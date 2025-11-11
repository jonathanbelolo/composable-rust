//! Data manipulation tools for JSON queries and string transformations

use composable_rust_core::agent::{Tool, ToolError, ToolExecutorFn, ToolResult};
use serde_json::json;
use std::sync::Arc;

/// Create the `json_query` tool
///
/// Query JSON data using JSONPath expressions.
///
/// Returns JSON:
/// ```json
/// {
///   "results": [...matching values...]
/// }
/// ```
#[must_use]
pub fn json_query_tool() -> (Tool, ToolExecutorFn) {
    let tool = Tool {
        name: "json_query".to_string(),
        description: "Query JSON data using JSONPath expressions".to_string(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "data": {
                    "type": "string",
                    "description": "JSON data to query"
                },
                "query": {
                    "type": "string",
                    "description": "JSONPath query expression (e.g., '$.users[*].name')"
                }
            },
            "required": ["data", "query"]
        }),
    };

    let executor = Arc::new(|input: String| {
        Box::pin(async move {
            let parsed: serde_json::Value = serde_json::from_str(&input).map_err(|e| {
                ToolError {
                    message: format!("Invalid input JSON: {e}"),
                }
            })?;

            let data_str = parsed["data"]
                .as_str()
                .ok_or_else(|| ToolError {
                    message: "Missing 'data' field".to_string(),
                })?;

            let query = parsed["query"]
                .as_str()
                .ok_or_else(|| ToolError {
                    message: "Missing 'query' field".to_string(),
                })?;

            // Parse JSON data
            let data: serde_json::Value = serde_json::from_str(data_str).map_err(|e| {
                ToolError {
                    message: format!("Invalid JSON data: {e}"),
                }
            })?;

            // Execute JSONPath query (blocking operation)
            let data_owned = data.clone();
            let query_owned = query.to_string();
            let results = tokio::task::spawn_blocking(move || {
                use jsonpath_rust::JsonPathQuery;
                data_owned.path(&query_owned)
            })
            .await
            .map_err(|e| ToolError {
                message: format!("Failed to spawn query task: {e}"),
            })?
            .map_err(|e| ToolError {
                message: format!("JSONPath query error: {e}"),
            })?;

            let output = json!({
                "results": results
            });

            Ok(output.to_string())
        }) as std::pin::Pin<
            Box<dyn std::future::Future<Output = ToolResult> + Send>,
        >
    }) as ToolExecutorFn;

    (tool, executor)
}

/// Create the `string_transform` tool
///
/// Transform strings with common operations.
///
/// Supported operations:
/// - `uppercase`: Convert to uppercase
/// - `lowercase`: Convert to lowercase
/// - `trim`: Remove leading/trailing whitespace
/// - `trim_start`: Remove leading whitespace
/// - `trim_end`: Remove trailing whitespace
/// - `reverse`: Reverse string
/// - `length`: Get string length
///
/// Returns JSON:
/// ```json
/// {
///   "result": "transformed string"
/// }
/// ```
/// or for length:
/// ```json
/// {
///   "length": 42
/// }
/// ```
#[must_use]
pub fn string_transform_tool() -> (Tool, ToolExecutorFn) {
    let tool = Tool {
        name: "string_transform".to_string(),
        description: "Transform strings with common operations (uppercase, lowercase, trim, reverse, length)".to_string(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "text": {
                    "type": "string",
                    "description": "Text to transform"
                },
                "operation": {
                    "type": "string",
                    "enum": ["uppercase", "lowercase", "trim", "trim_start", "trim_end", "reverse", "length"],
                    "description": "Transformation to apply"
                }
            },
            "required": ["text", "operation"]
        }),
    };

    let executor = Arc::new(|input: String| {
        Box::pin(async move {
            let parsed: serde_json::Value = serde_json::from_str(&input).map_err(|e| {
                ToolError {
                    message: format!("Invalid input JSON: {e}"),
                }
            })?;

            let text = parsed["text"]
                .as_str()
                .ok_or_else(|| ToolError {
                    message: "Missing 'text' field".to_string(),
                })?;

            let operation = parsed["operation"]
                .as_str()
                .ok_or_else(|| ToolError {
                    message: "Missing 'operation' field".to_string(),
                })?;

            let output = match operation {
                "uppercase" => json!({ "result": text.to_uppercase() }),
                "lowercase" => json!({ "result": text.to_lowercase() }),
                "trim" => json!({ "result": text.trim() }),
                "trim_start" => json!({ "result": text.trim_start() }),
                "trim_end" => json!({ "result": text.trim_end() }),
                "reverse" => json!({ "result": text.chars().rev().collect::<String>() }),
                "length" => json!({ "length": text.len() }),
                _ => {
                    return Err(ToolError {
                        message: format!("Unknown operation: {operation}"),
                    });
                }
            };

            Ok(output.to_string())
        }) as std::pin::Pin<
            Box<dyn std::future::Future<Output = ToolResult> + Send>,
        >
    }) as ToolExecutorFn;

    (tool, executor)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_json_query_tool_schema() {
        let (tool, _executor) = json_query_tool();
        assert_eq!(tool.name, "json_query");
        assert!(tool.input_schema.is_object());
    }

    #[test]
    fn test_string_transform_tool_schema() {
        let (tool, _executor) = string_transform_tool();
        assert_eq!(tool.name, "string_transform");
        assert!(tool.input_schema.is_object());
    }

    #[tokio::test]
    async fn test_json_query_simple() {
        let (_tool, executor) = json_query_tool();

        let input = json!({
            "data": r#"{"users": [{"name": "Alice"}, {"name": "Bob"}]}"#,
            "query": "$.users[*].name"
        })
        .to_string();

        let result = executor(input).await;
        assert!(result.is_ok());

        let output: serde_json::Value = serde_json::from_str(&result.expect("should succeed")).expect("valid JSON");
        assert!(output["results"].is_array());
    }

    #[tokio::test]
    async fn test_string_transform_uppercase() {
        let (_tool, executor) = string_transform_tool();

        let input = json!({
            "text": "hello",
            "operation": "uppercase"
        })
        .to_string();

        let result = executor(input).await;
        assert!(result.is_ok());

        let output: serde_json::Value = serde_json::from_str(&result.expect("should succeed")).expect("valid JSON");
        assert_eq!(output["result"], "HELLO");
    }

    #[tokio::test]
    async fn test_string_transform_lowercase() {
        let (_tool, executor) = string_transform_tool();

        let input = json!({
            "text": "HELLO",
            "operation": "lowercase"
        })
        .to_string();

        let result = executor(input).await;
        assert!(result.is_ok());

        let output: serde_json::Value = serde_json::from_str(&result.expect("should succeed")).expect("valid JSON");
        assert_eq!(output["result"], "hello");
    }

    #[tokio::test]
    async fn test_string_transform_trim() {
        let (_tool, executor) = string_transform_tool();

        let input = json!({
            "text": "  hello  ",
            "operation": "trim"
        })
        .to_string();

        let result = executor(input).await;
        assert!(result.is_ok());

        let output: serde_json::Value = serde_json::from_str(&result.expect("should succeed")).expect("valid JSON");
        assert_eq!(output["result"], "hello");
    }

    #[tokio::test]
    async fn test_string_transform_reverse() {
        let (_tool, executor) = string_transform_tool();

        let input = json!({
            "text": "hello",
            "operation": "reverse"
        })
        .to_string();

        let result = executor(input).await;
        assert!(result.is_ok());

        let output: serde_json::Value = serde_json::from_str(&result.expect("should succeed")).expect("valid JSON");
        assert_eq!(output["result"], "olleh");
    }

    #[tokio::test]
    async fn test_string_transform_length() {
        let (_tool, executor) = string_transform_tool();

        let input = json!({
            "text": "hello",
            "operation": "length"
        })
        .to_string();

        let result = executor(input).await;
        assert!(result.is_ok());

        let output: serde_json::Value = serde_json::from_str(&result.expect("should succeed")).expect("valid JSON");
        assert_eq!(output["length"], 5);
    }

    #[tokio::test]
    async fn test_string_transform_unknown_operation() {
        let (_tool, executor) = string_transform_tool();

        let input = json!({
            "text": "hello",
            "operation": "unknown"
        })
        .to_string();

        let result = executor(input).await;
        assert!(result.is_err());
        assert!(result
            .expect_err("should fail")
            .message
            .contains("Unknown operation"));
    }
}
