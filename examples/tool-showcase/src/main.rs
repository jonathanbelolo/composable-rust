//! Tool Showcase Example
//!
//! Demonstrates all built-in tools from composable-rust-tools:
//! - HTTP tools (`http_request`, `http_get`, `http_get_markdown`)
//! - Time tools (`current_time`)
//! - Calculation tools (calculate)
//! - Data tools (`json_query`, `string_transform`)
//! - Todo tools (`todo_add`, `todo_list`, `todo_complete`, `todo_delete`)
//! - Mock tools (`memory_search`, `web_search`)
//! - Tool registry for dynamic tool management
//! - Retry policies and timeout handling

use composable_rust_tools::{
    calculation::calculate_tool,
    data::{json_query_tool, string_transform_tool},
    http::{http_get_markdown_tool, http_get_tool, http_request_tool},
    mock::{memory_search_tool, web_search_tool},
    time::current_time_tool,
    todo::{todo_add_tool, todo_complete_tool, todo_delete_tool, todo_list_tool, TodoStore},
    ToolConfig, ToolRegistry,
};
use serde_json::json;
use std::time::Duration;

#[tokio::main]
#[allow(clippy::too_many_lines)] // Example showcase with many demonstrations
#[allow(clippy::expect_used)] // Example code for demonstration
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    tracing_subscriber::fmt::init();

    println!("=== Composable Rust Tools Showcase ===\n");

    // Create tool registry
    let registry = ToolRegistry::new();

    // Register HTTP tools
    println!("📡 Registering HTTP tools...");
    let (http_request, http_request_exec) = http_request_tool();
    let (http_get, http_get_exec) = http_get_tool();
    let (http_get_markdown, http_get_markdown_exec) = http_get_markdown_tool();
    registry.register(http_request, http_request_exec);
    registry.register(http_get, http_get_exec);
    registry.register(http_get_markdown, http_get_markdown_exec);

    // Register time tools
    println!("⏰ Registering time tools...");
    let (current_time, current_time_exec) = current_time_tool();
    registry.register(current_time, current_time_exec);

    // Register calculation tools
    println!("🧮 Registering calculation tools...");
    let (calculate, calculate_exec) = calculate_tool();
    registry.register(calculate, calculate_exec);

    // Register data tools
    println!("📊 Registering data tools...");
    let (json_query, json_query_exec) = json_query_tool();
    let (string_transform, string_transform_exec) = string_transform_tool();
    registry.register(json_query, json_query_exec);
    registry.register(string_transform, string_transform_exec);

    // Register todo tools (with shared store)
    println!("✅ Registering todo tools...");
    let todo_store = TodoStore::new();
    let (todo_add, todo_add_exec) = todo_add_tool(todo_store.clone());
    let (todo_list, todo_list_exec) = todo_list_tool(todo_store.clone());
    let (todo_complete, todo_complete_exec) = todo_complete_tool(todo_store.clone());
    let (todo_delete, todo_delete_exec) = todo_delete_tool(todo_store);
    registry.register(todo_add, todo_add_exec);
    registry.register(todo_list, todo_list_exec);
    registry.register(todo_complete, todo_complete_exec);
    registry.register(todo_delete, todo_delete_exec);

    // Register mock tools
    println!("🔍 Registering mock tools...");
    let (memory_search, memory_search_exec) = memory_search_tool();
    let (web_search, web_search_exec) = web_search_tool();
    registry.register(memory_search, memory_search_exec);
    registry.register(web_search, web_search_exec);

    println!("\n✨ Registered {} tools\n", registry.count());

    // Demo 1: Time tool
    println!("=== Demo 1: Current Time ===");
    let time_result = registry
        .execute(
            "current_time",
            json!({
                "timezone": "America/New_York"
            })
            .to_string(),
        )
        .await?;
    println!("Result: {}\n", pretty_json(&time_result));

    // Demo 2: Calculation
    println!("=== Demo 2: Calculate ===");
    let calc_result = registry
        .execute(
            "calculate",
            json!({
                "expression": "sqrt(16) + 2^3"
            })
            .to_string(),
        )
        .await?;
    println!("Result: {}\n", pretty_json(&calc_result));

    // Demo 3: String transformation
    println!("=== Demo 3: String Transform ===");
    let transform_result = registry
        .execute(
            "string_transform",
            json!({
                "text": "hello world",
                "operation": "uppercase"
            })
            .to_string(),
        )
        .await?;
    println!("Result: {}\n", pretty_json(&transform_result));

    // Demo 4: JSON query
    println!("=== Demo 4: JSON Query ===");
    let json_data = json!({
        "users": [
            {"name": "Alice", "age": 30},
            {"name": "Bob", "age": 25}
        ]
    });
    let query_result = registry
        .execute(
            "json_query",
            json!({
                "data": json_data.to_string(),
                "query": "$.users[*].name"
            })
            .to_string(),
        )
        .await?;
    println!("Result: {}\n", pretty_json(&query_result));

    // Demo 5: Todo workflow
    println!("=== Demo 5: Todo Workflow ===");

    println!("Adding todo...");
    let add_result = registry
        .execute(
            "todo_add",
            json!({
                "title": "Implement Phase 8.2 tools"
            })
            .to_string(),
        )
        .await?;
    let added: serde_json::Value = serde_json::from_str(&add_result)?;
    let todo_id = added["id"].as_u64().expect("should have ID");
    println!("Added: {}\n", pretty_json(&add_result));

    println!("Listing todos...");
    let list_result = registry.execute("todo_list", json!({}).to_string()).await?;
    println!("List: {}\n", pretty_json(&list_result));

    println!("Completing todo...");
    let complete_result = registry
        .execute(
            "todo_complete",
            json!({
                "id": todo_id
            })
            .to_string(),
        )
        .await?;
    println!("Completed: {}\n", pretty_json(&complete_result));

    // Demo 6: Mock search tools
    println!("=== Demo 6: Mock Search ===");

    println!("Memory search:");
    let memory_result = registry
        .execute(
            "memory_search",
            json!({
                "query": "weather"
            })
            .to_string(),
        )
        .await?;
    println!("Result: {}\n", pretty_json(&memory_result));

    println!("Web search:");
    let web_result = registry
        .execute(
            "web_search",
            json!({
                "query": "Rust programming"
            })
            .to_string(),
        )
        .await?;
    println!("Result: {}\n", pretty_json(&web_result));

    // Demo 7: Retry with timeout
    println!("=== Demo 7: Retry Policy ===");
    let config = ToolConfig::fixed_retry(3, Duration::from_millis(100))
        .with_timeout(Duration::from_secs(5));

    let retry_result = composable_rust_tools::execute_with_retry(&config, || async {
        
        registry
            .execute(
                "calculate",
                json!({
                    "expression": "42 * 2"
                })
                .to_string(),
            )
            .await
    })
    .await?;
    println!("Result with retry: {}\n", pretty_json(&retry_result));

    // List all registered tools
    println!("=== Registered Tools ===");
    for tool_name in registry.list_tools() {
        if let Some(tool) = registry.get_tool(&tool_name) {
            println!("  - {}: {}", tool.name, tool.description);
        }
    }

    println!("\n✨ Showcase complete!");

    Ok(())
}

/// Pretty-print JSON
fn pretty_json(json_str: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(json_str) {
        Ok(value) => serde_json::to_string_pretty(&value).unwrap_or_else(|_| json_str.to_string()),
        Err(_) => json_str.to_string(),
    }
}
