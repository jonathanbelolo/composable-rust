//! Mock tools for testing and demonstration
//!
//! These tools provide mock/simulated responses useful for:
//! - Testing agent behavior without external dependencies
//! - Demonstrating tool use in examples
//! - Development and debugging

use composable_rust_core::agent::{Tool, ToolError, ToolExecutorFn, ToolResult};
use serde_json::json;
use std::sync::Arc;

/// Create the `memory_search` mock tool
///
/// Simulates searching through conversation memory.
/// Returns mock search results based on query keywords.
///
/// Returns JSON:
/// ```json
/// {
///   "query": "user search query",
///   "results": [
///     {"relevance": 0.95, "text": "matching memory..."},
///     {"relevance": 0.82, "text": "another memory..."}
///   ]
/// }
/// ```
#[must_use]
pub fn memory_search_tool() -> (Tool, ToolExecutorFn) {
    let tool = Tool {
        name: "memory_search".to_string(),
        description: "Search through conversation memory (mock implementation)".to_string(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query"
                }
            },
            "required": ["query"]
        }),
    };

    let executor = Arc::new(|input: String| {
        Box::pin(async move {
            let parsed: serde_json::Value = serde_json::from_str(&input).map_err(|e| {
                ToolError {
                    message: format!("Invalid input JSON: {e}"),
                }
            })?;

            let query = parsed["query"]
                .as_str()
                .ok_or_else(|| ToolError {
                    message: "Missing 'query' field".to_string(),
                })?;

            // Generate mock results based on query
            let results = if query.to_lowercase().contains("weather") {
                vec![
                    json!({
                        "relevance": 0.95,
                        "text": "User asked about weather in San Francisco earlier.",
                        "timestamp": "2025-01-15T10:00:00Z"
                    }),
                    json!({
                        "relevance": 0.82,
                        "text": "Discussion about weather patterns and climate change.",
                        "timestamp": "2025-01-14T15:30:00Z"
                    }),
                ]
            } else if query.to_lowercase().contains("code") {
                vec![
                    json!({
                        "relevance": 0.88,
                        "text": "User requested help with Rust async code.",
                        "timestamp": "2025-01-15T09:15:00Z"
                    }),
                    json!({
                        "relevance": 0.75,
                        "text": "Discussion about code architecture patterns.",
                        "timestamp": "2025-01-13T14:20:00Z"
                    }),
                ]
            } else {
                vec![json!({
                    "relevance": 0.50,
                    "text": format!("No specific memories found for query: {query}"),
                    "timestamp": "2025-01-15T12:00:00Z"
                })]
            };

            let output = json!({
                "query": query,
                "results": results
            });

            Ok(output.to_string())
        }) as std::pin::Pin<
            Box<dyn std::future::Future<Output = ToolResult> + Send>,
        >
    }) as ToolExecutorFn;

    (tool, executor)
}

/// Create the `web_search` mock tool
///
/// Simulates web search results.
/// Returns mock search results based on query keywords.
///
/// Returns JSON:
/// ```json
/// {
///   "query": "user search query",
///   "results": [
///     {
///       "title": "Result Title",
///       "url": "https://example.com",
///       "snippet": "Brief description...",
///       "relevance": 0.92
///     }
///   ]
/// }
/// ```
#[must_use]
pub fn web_search_tool() -> (Tool, ToolExecutorFn) {
    let tool = Tool {
        name: "web_search".to_string(),
        description: "Search the web (mock implementation)".to_string(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query"
                }
            },
            "required": ["query"]
        }),
    };

    let executor = Arc::new(|input: String| {
        Box::pin(async move {
            let parsed: serde_json::Value = serde_json::from_str(&input).map_err(|e| {
                ToolError {
                    message: format!("Invalid input JSON: {e}"),
                }
            })?;

            let query = parsed["query"]
                .as_str()
                .ok_or_else(|| ToolError {
                    message: "Missing 'query' field".to_string(),
                })?;

            // Generate mock results based on query
            let results = if query.to_lowercase().contains("rust") {
                vec![
                    json!({
                        "title": "The Rust Programming Language",
                        "url": "https://www.rust-lang.org/",
                        "snippet": "A language empowering everyone to build reliable and efficient software.",
                        "relevance": 0.98
                    }),
                    json!({
                        "title": "Rust by Example",
                        "url": "https://doc.rust-lang.org/rust-by-example/",
                        "snippet": "Rust by Example (RBE) is a collection of runnable examples.",
                        "relevance": 0.92
                    }),
                    json!({
                        "title": "The Rust Book",
                        "url": "https://doc.rust-lang.org/book/",
                        "snippet": "The official Rust programming language book.",
                        "relevance": 0.95
                    }),
                ]
            } else if query.to_lowercase().contains("weather") {
                vec![
                    json!({
                        "title": "Weather.com - Local Weather Forecast",
                        "url": "https://weather.com/",
                        "snippet": "Get current weather, hourly forecasts, and weather maps.",
                        "relevance": 0.91
                    }),
                    json!({
                        "title": "National Weather Service",
                        "url": "https://www.weather.gov/",
                        "snippet": "Official weather forecasts, warnings, and meteorological information.",
                        "relevance": 0.89
                    }),
                ]
            } else {
                vec![
                    json!({
                        "title": format!("Search results for: {query}"),
                        "url": "https://example.com/search",
                        "snippet": format!("Mock search results for query: {query}"),
                        "relevance": 0.70
                    }),
                ]
            };

            let output = json!({
                "query": query,
                "results": results
            });

            Ok(output.to_string())
        }) as std::pin::Pin<
            Box<dyn std::future::Future<Output = ToolResult> + Send>,
        >
    }) as ToolExecutorFn;

    (tool, executor)
}

#[cfg(test)]
#[allow(clippy::expect_used)] // Test code can use expect
mod tests {
    use super::*;

    #[test]
    fn test_memory_search_tool_schema() {
        let (tool, _executor) = memory_search_tool();
        assert_eq!(tool.name, "memory_search");
        assert!(tool.input_schema.is_object());
    }

    #[test]
    fn test_web_search_tool_schema() {
        let (tool, _executor) = web_search_tool();
        assert_eq!(tool.name, "web_search");
        assert!(tool.input_schema.is_object());
    }

    #[tokio::test]
    async fn test_memory_search_weather_query() {
        let (_tool, executor) = memory_search_tool();

        let input = json!({
            "query": "weather"
        })
        .to_string();

        let result = executor(input).await;
        assert!(result.is_ok());

        let output: serde_json::Value = serde_json::from_str(&result.expect("should succeed")).expect("valid JSON");
        assert_eq!(output["query"], "weather");
        assert!(output["results"].is_array());
        assert!(!output["results"].as_array().expect("is array").is_empty());
    }

    #[tokio::test]
    async fn test_web_search_rust_query() {
        let (_tool, executor) = web_search_tool();

        let input = json!({
            "query": "Rust programming"
        })
        .to_string();

        let result = executor(input).await;
        assert!(result.is_ok());

        let output: serde_json::Value = serde_json::from_str(&result.expect("should succeed")).expect("valid JSON");
        assert_eq!(output["query"], "Rust programming");
        assert!(output["results"].is_array());
        assert!(!output["results"].as_array().expect("is array").is_empty());

        // Verify result structure
        let first_result = &output["results"][0];
        assert!(first_result["title"].is_string());
        assert!(first_result["url"].is_string());
        assert!(first_result["snippet"].is_string());
        assert!(first_result["relevance"].is_number());
    }

    #[tokio::test]
    async fn test_memory_search_generic_query() {
        let (_tool, executor) = memory_search_tool();

        let input = json!({
            "query": "something random"
        })
        .to_string();

        let result = executor(input).await;
        assert!(result.is_ok());

        let output: serde_json::Value = serde_json::from_str(&result.expect("should succeed")).expect("valid JSON");
        assert!(output["results"].is_array());
        assert_eq!(output["results"].as_array().expect("is array").len(), 1);
    }
}
