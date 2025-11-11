//! Time tools for getting current time in various formats

use chrono::{Local, TimeZone, Utc};
use chrono_tz::Tz;
use composable_rust_core::agent::{Tool, ToolError, ToolExecutorFn, ToolResult};
use serde_json::json;
use std::sync::Arc;

/// Create the `current_time` tool
///
/// Get current time in various formats and timezones.
///
/// Returns JSON:
/// ```json
/// {
///   "utc": "2025-01-15T10:30:00Z",
///   "local": "2025-01-15T02:30:00-08:00",
///   "timezone": "America/Los_Angeles",
///   "unix_timestamp": 1705315800
/// }
/// ```
#[must_use]
pub fn current_time_tool() -> (Tool, ToolExecutorFn) {
    let tool = Tool {
        name: "current_time".to_string(),
        description: "Get current time in UTC, local timezone, or specified timezone".to_string(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "timezone": {
                    "type": "string",
                    "description": "Optional timezone (e.g. 'America/New_York', 'Europe/London'). Defaults to UTC."
                }
            }
        }),
    };

    let executor = Arc::new(|input: String| {
        Box::pin(async move {
            let parsed: serde_json::Value = serde_json::from_str(&input).map_err(|e| {
                ToolError {
                    message: format!("Invalid input JSON: {e}"),
                }
            })?;

            let now_utc = Utc::now();
            let now_local = Local::now();

            // Parse optional timezone
            let timezone_str = parsed["timezone"].as_str();
            let timezone_time = if let Some(tz_str) = timezone_str {
                let tz: Tz = tz_str.parse().map_err(|_| ToolError {
                    message: format!("Invalid timezone: {tz_str}"),
                })?;
                Some(tz.from_utc_datetime(&now_utc.naive_utc()))
            } else {
                None
            };

            let result = json!({
                "utc": now_utc.to_rfc3339(),
                "local": now_local.to_rfc3339(),
                "timezone": timezone_time.map(|dt| dt.to_rfc3339()),
                "timezone_name": timezone_str,
                "unix_timestamp": now_utc.timestamp(),
            });

            Ok(result.to_string())
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
    fn test_current_time_tool_schema() {
        let (tool, _executor) = current_time_tool();
        assert_eq!(tool.name, "current_time");
        assert!(tool.input_schema.is_object());
    }

    #[tokio::test]
    async fn test_current_time_utc() {
        let (_tool, executor) = current_time_tool();

        let input = json!({}).to_string();

        let result = executor(input).await;
        assert!(result.is_ok());

        let output: serde_json::Value = serde_json::from_str(&result.expect("should succeed")).expect("valid JSON");
        assert!(output["utc"].is_string());
        assert!(output["unix_timestamp"].is_number());
    }

    #[tokio::test]
    async fn test_current_time_with_timezone() {
        let (_tool, executor) = current_time_tool();

        let input = json!({
            "timezone": "America/New_York"
        })
        .to_string();

        let result = executor(input).await;
        assert!(result.is_ok());

        let output: serde_json::Value = serde_json::from_str(&result.expect("should succeed")).expect("valid JSON");
        assert!(output["timezone"].is_string());
        assert_eq!(output["timezone_name"], "America/New_York");
    }

    #[tokio::test]
    async fn test_current_time_invalid_timezone() {
        let (_tool, executor) = current_time_tool();

        let input = json!({
            "timezone": "Invalid/Timezone"
        })
        .to_string();

        let result = executor(input).await;
        assert!(result.is_err());
        assert!(result
            .expect_err("should fail")
            .message
            .contains("Invalid timezone"));
    }
}
