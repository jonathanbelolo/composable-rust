//! HTTP tools for making web requests
//!
//! Provides three HTTP tools:
//! - `http_request`: Full HTTP request with method, headers, body
//! - `http_get`: Simple GET request
//! - `http_get_markdown`: GET request with HTML→Markdown conversion

use composable_rust_core::agent::{Tool, ToolError, ToolExecutorFn, ToolResult};
use futures::StreamExt;
use serde_json::json;
use std::sync::Arc;

/// Maximum response size (50MB)
const MAX_RESPONSE_SIZE: usize = 50 * 1024 * 1024;

/// Create the `http_request` tool
///
/// Full-featured HTTP request with method, URL, headers, and body.
///
/// Returns JSON:
/// ```json
/// {
///   "status": 200,
///   "headers": {"content-type": "application/json"},
///   "body": "response body"
/// }
/// ```
#[must_use]
pub fn http_request_tool() -> (Tool, ToolExecutorFn) {
    let tool = Tool {
        name: "http_request".to_string(),
        description: "Make an HTTP request with custom method, headers, and body".to_string(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "method": {
                    "type": "string",
                    "enum": ["GET", "POST", "PUT", "DELETE", "PATCH", "HEAD"],
                    "description": "HTTP method"
                },
                "url": {
                    "type": "string",
                    "description": "Target URL (must be http:// or https://)"
                },
                "headers": {
                    "type": "object",
                    "description": "Optional HTTP headers",
                    "additionalProperties": {"type": "string"}
                },
                "body": {
                    "type": "string",
                    "description": "Optional request body"
                }
            },
            "required": ["method", "url"]
        }),
    };

    let executor = Arc::new(|input: String| {
        Box::pin(async move {
            let parsed: serde_json::Value = serde_json::from_str(&input).map_err(|e| {
                ToolError {
                    message: format!("Invalid input JSON: {e}"),
                }
            })?;

            let method = parsed["method"]
                .as_str()
                .ok_or_else(|| ToolError {
                    message: "Missing 'method' field".to_string(),
                })?;

            let url = parsed["url"]
                .as_str()
                .ok_or_else(|| ToolError {
                    message: "Missing 'url' field".to_string(),
                })?;

            // Security: Only allow http:// and https://
            if !url.starts_with("http://") && !url.starts_with("https://") {
                return Err(ToolError {
                    message: "URL must start with http:// or https://".to_string(),
                });
            }

            let client = reqwest::Client::new();
            let mut request = match method {
                "GET" => client.get(url),
                "POST" => client.post(url),
                "PUT" => client.put(url),
                "DELETE" => client.delete(url),
                "PATCH" => client.patch(url),
                "HEAD" => client.head(url),
                _ => {
                    return Err(ToolError {
                        message: format!("Unsupported method: {method}"),
                    });
                }
            };

            // Add headers if provided
            if let Some(headers_obj) = parsed["headers"].as_object() {
                for (key, value) in headers_obj {
                    if let Some(value_str) = value.as_str() {
                        request = request.header(key, value_str);
                    }
                }
            }

            // Add body if provided
            if let Some(body) = parsed["body"].as_str() {
                request = request.body(body.to_string());
            }

            // Execute request
            let response = request.send().await.map_err(|e| ToolError {
                message: format!("HTTP request failed: {e}"),
            })?;

            let status = response.status().as_u16();
            let headers: serde_json::Map<String, serde_json::Value> = response
                .headers()
                .iter()
                .map(|(k, v)| {
                    (
                        k.as_str().to_string(),
                        serde_json::Value::String(
                            v.to_str().unwrap_or("<invalid>").to_string(),
                        ),
                    )
                })
                .collect();

            // Stream response with size limit
            let mut body_bytes = Vec::new();
            let mut stream = response.bytes_stream();

            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|e| ToolError {
                    message: format!("Failed to read response: {e}"),
                })?;

                if body_bytes.len() + chunk.len() > MAX_RESPONSE_SIZE {
                    return Err(ToolError {
                        message: format!("Response too large (>{MAX_RESPONSE_SIZE} bytes)"),
                    });
                }

                body_bytes.extend_from_slice(&chunk);
            }

            let body = String::from_utf8_lossy(&body_bytes).to_string();

            let result = json!({
                "status": status,
                "headers": headers,
                "body": body
            });

            Ok(result.to_string())
        }) as std::pin::Pin<
            Box<dyn std::future::Future<Output = ToolResult> + Send>,
        >
    }) as ToolExecutorFn;

    (tool, executor)
}

/// Create the `http_get` tool
///
/// Simple HTTP GET request (convenience wrapper around `http_request`).
///
/// Returns JSON:
/// ```json
/// {
///   "status": 200,
///   "headers": {"content-type": "text/html"},
///   "body": "response body"
/// }
/// ```
#[must_use]
pub fn http_get_tool() -> (Tool, ToolExecutorFn) {
    let tool = Tool {
        name: "http_get".to_string(),
        description: "Make a simple HTTP GET request".to_string(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "Target URL (must be http:// or https://)"
                }
            },
            "required": ["url"]
        }),
    };

    let executor = Arc::new(|input: String| {
        Box::pin(async move {
            let parsed: serde_json::Value = serde_json::from_str(&input).map_err(|e| {
                ToolError {
                    message: format!("Invalid input JSON: {e}"),
                }
            })?;

            let url = parsed["url"]
                .as_str()
                .ok_or_else(|| ToolError {
                    message: "Missing 'url' field".to_string(),
                })?;

            // Security: Only allow http:// and https://
            if !url.starts_with("http://") && !url.starts_with("https://") {
                return Err(ToolError {
                    message: "URL must start with http:// or https://".to_string(),
                });
            }

            let client = reqwest::Client::new();
            let response = client.get(url).send().await.map_err(|e| ToolError {
                message: format!("HTTP request failed: {e}"),
            })?;

            let status = response.status().as_u16();
            let headers: serde_json::Map<String, serde_json::Value> = response
                .headers()
                .iter()
                .map(|(k, v)| {
                    (
                        k.as_str().to_string(),
                        serde_json::Value::String(
                            v.to_str().unwrap_or("<invalid>").to_string(),
                        ),
                    )
                })
                .collect();

            // Stream response with size limit
            let mut body_bytes = Vec::new();
            let mut stream = response.bytes_stream();

            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|e| ToolError {
                    message: format!("Failed to read response: {e}"),
                })?;

                if body_bytes.len() + chunk.len() > MAX_RESPONSE_SIZE {
                    return Err(ToolError {
                        message: format!("Response too large (>{MAX_RESPONSE_SIZE} bytes)"),
                    });
                }

                body_bytes.extend_from_slice(&chunk);
            }

            let body = String::from_utf8_lossy(&body_bytes).to_string();

            let result = json!({
                "status": status,
                "headers": headers,
                "body": body
            });

            Ok(result.to_string())
        }) as std::pin::Pin<
            Box<dyn std::future::Future<Output = ToolResult> + Send>,
        >
    }) as ToolExecutorFn;

    (tool, executor)
}

/// Create the `http_get_markdown` tool
///
/// HTTP GET request with HTML→Markdown conversion for token efficiency.
///
/// Returns JSON:
/// ```json
/// {
///   "status": 200,
///   "headers": {"content-type": "text/html"},
///   "markdown": "# Page Title\n\nContent...",
///   "original_size": 50000,
///   "markdown_size": 15000
/// }
/// ```
#[must_use]
pub fn http_get_markdown_tool() -> (Tool, ToolExecutorFn) {
    let tool = Tool {
        name: "http_get_markdown".to_string(),
        description:
            "Make HTTP GET request and convert HTML to Markdown for token efficiency".to_string(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "Target URL (must be http:// or https://)"
                }
            },
            "required": ["url"]
        }),
    };

    let executor = Arc::new(|input: String| {
        Box::pin(async move {
            let parsed: serde_json::Value = serde_json::from_str(&input).map_err(|e| {
                ToolError {
                    message: format!("Invalid input JSON: {e}"),
                }
            })?;

            let url = parsed["url"]
                .as_str()
                .ok_or_else(|| ToolError {
                    message: "Missing 'url' field".to_string(),
                })?;

            // Security: Only allow http:// and https://
            if !url.starts_with("http://") && !url.starts_with("https://") {
                return Err(ToolError {
                    message: "URL must start with http:// or https://".to_string(),
                });
            }

            let client = reqwest::Client::new();
            let response = client.get(url).send().await.map_err(|e| ToolError {
                message: format!("HTTP request failed: {e}"),
            })?;

            let status = response.status().as_u16();
            let headers: serde_json::Map<String, serde_json::Value> = response
                .headers()
                .iter()
                .map(|(k, v)| {
                    (
                        k.as_str().to_string(),
                        serde_json::Value::String(
                            v.to_str().unwrap_or("<invalid>").to_string(),
                        ),
                    )
                })
                .collect();

            // Stream response with size limit
            let mut body_bytes = Vec::new();
            let mut stream = response.bytes_stream();

            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|e| ToolError {
                    message: format!("Failed to read response: {e}"),
                })?;

                if body_bytes.len() + chunk.len() > MAX_RESPONSE_SIZE {
                    return Err(ToolError {
                        message: format!("Response too large (>{MAX_RESPONSE_SIZE} bytes)"),
                    });
                }

                body_bytes.extend_from_slice(&chunk);
            }

            let html = String::from_utf8_lossy(&body_bytes).to_string();
            let original_size = html.len();

            // Convert HTML to Markdown
            let markdown = html2md::parse_html(&html);
            let markdown_size = markdown.len();

            let result = json!({
                "status": status,
                "headers": headers,
                "markdown": markdown,
                "original_size": original_size,
                "markdown_size": markdown_size
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
    fn test_http_request_tool_schema() {
        let (tool, _executor) = http_request_tool();
        assert_eq!(tool.name, "http_request");
        assert!(tool.input_schema.is_object());
    }

    #[test]
    fn test_http_get_tool_schema() {
        let (tool, _executor) = http_get_tool();
        assert_eq!(tool.name, "http_get");
        assert!(tool.input_schema.is_object());
    }

    #[test]
    fn test_http_get_markdown_tool_schema() {
        let (tool, _executor) = http_get_markdown_tool();
        assert_eq!(tool.name, "http_get_markdown");
        assert!(tool.input_schema.is_object());
    }

    #[tokio::test]
    async fn test_http_request_rejects_invalid_url() {
        let (_tool, executor) = http_request_tool();

        let input = json!({
            "method": "GET",
            "url": "file:///etc/passwd"
        })
        .to_string();

        let result = executor(input).await;
        assert!(result.is_err());
        assert!(result
            .expect_err("should fail")
            .message
            .contains("http://"));
    }

    #[tokio::test]
    async fn test_http_get_rejects_invalid_url() {
        let (_tool, executor) = http_get_tool();

        let input = json!({
            "url": "ftp://example.com"
        })
        .to_string();

        let result = executor(input).await;
        assert!(result.is_err());
        assert!(result
            .expect_err("should fail")
            .message
            .contains("http://"));
    }
}
