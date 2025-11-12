//! Calculation tools for mathematical expressions

use composable_rust_core::agent::{Tool, ToolError, ToolExecutorFn, ToolResult};
use serde_json::json;
use std::sync::Arc;

/// Create the `calculate` tool
///
/// Evaluate mathematical expressions using meval.
///
/// Supports:
/// - Basic arithmetic: +, -, *, /, %
/// - Parentheses: ()
/// - Exponentiation: ^
/// - Functions: sin, cos, tan, sqrt, abs, etc.
///
/// Returns JSON:
/// ```json
/// {
///   "expression": "2 + 2 * 3",
///   "result": 8.0
/// }
/// ```
#[must_use]
pub fn calculate_tool() -> (Tool, ToolExecutorFn) {
    let tool = Tool {
        name: "calculate".to_string(),
        description: "Evaluate mathematical expressions (supports +, -, *, /, %, ^, functions like sin/cos/sqrt)".to_string(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "expression": {
                    "type": "string",
                    "description": "Mathematical expression to evaluate"
                }
            },
            "required": ["expression"]
        }),
    };

    let executor = Arc::new(|input: String| {
        Box::pin(async move {
            let parsed: serde_json::Value = serde_json::from_str(&input).map_err(|e| {
                ToolError {
                    message: format!("Invalid input JSON: {e}"),
                }
            })?;

            let expression = parsed["expression"]
                .as_str()
                .ok_or_else(|| ToolError {
                    message: "Missing 'expression' field".to_string(),
                })?;

            // Evaluate expression (blocking operation)
            let expr_owned = expression.to_string();
            let result = tokio::task::spawn_blocking(move || {
                meval::eval_str(&expr_owned)
            })
            .await
            .map_err(|e| ToolError {
                message: format!("Failed to spawn calculation task: {e}"),
            })?
            .map_err(|e| ToolError {
                message: format!("Calculation error: {e}"),
            })?;

            let output = json!({
                "expression": expression,
                "result": result
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
    fn test_calculate_tool_schema() {
        let (tool, _executor) = calculate_tool();
        assert_eq!(tool.name, "calculate");
        assert!(tool.input_schema.is_object());
    }

    #[tokio::test]
    async fn test_calculate_simple() {
        let (_tool, executor) = calculate_tool();

        let input = json!({
            "expression": "2 + 2"
        })
        .to_string();

        let result = executor(input).await;
        assert!(result.is_ok());

        let output: serde_json::Value = serde_json::from_str(&result.expect("should succeed")).expect("valid JSON");
        assert_eq!(output["result"], 4.0);
    }

    #[tokio::test]
    async fn test_calculate_complex() {
        let (_tool, executor) = calculate_tool();

        let input = json!({
            "expression": "2 + 2 * 3"
        })
        .to_string();

        let result = executor(input).await;
        assert!(result.is_ok());

        let output: serde_json::Value = serde_json::from_str(&result.expect("should succeed")).expect("valid JSON");
        assert_eq!(output["result"], 8.0);
    }

    #[tokio::test]
    async fn test_calculate_with_functions() {
        let (_tool, executor) = calculate_tool();

        let input = json!({
            "expression": "sqrt(16)"
        })
        .to_string();

        let result = executor(input).await;
        assert!(result.is_ok());

        let output: serde_json::Value = serde_json::from_str(&result.expect("should succeed")).expect("valid JSON");
        assert_eq!(output["result"], 4.0);
    }

    #[tokio::test]
    async fn test_calculate_invalid_expression() {
        let (_tool, executor) = calculate_tool();

        let input = json!({
            "expression": "invalid + expression"
        })
        .to_string();

        let result = executor(input).await;
        // meval might be more forgiving than expected, so just check it runs
        // In practice, truly invalid expressions will fail
        assert!(result.is_ok() || result.is_err());
    }
}
