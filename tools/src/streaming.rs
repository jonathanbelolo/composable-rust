//! Streaming tool examples (Phase 8.3)
//!
//! This module provides example streaming tools that produce incremental output.
//! Streaming tools are useful for long-running operations where providing
//! intermediate results improves user experience.
//!
//! ## Streaming Pattern
//!
//! Streaming tools yield chunks of output as they become available:
//! 1. Tool starts execution
//! 2. As data becomes available, emit `ToolChunk` actions
//! 3. When complete, emit `ToolComplete` action
//!
//! ## Example: Progress Counter
//!
//! ```ignore
//! let (tool, executor) = progress_counter_tool();
//! // executor yields: ToolChunk("Progress: 10%"), ToolChunk("Progress: 20%"), ...
//! // Finally: ToolComplete(Ok("Completed: 100%"))
//! ```

use composable_rust_core::agent::{AgentAction, Tool, ToolError};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;

/// Type alias for streaming tool executor function
type StreamExecutor = Arc<
    dyn Fn(String, String) -> std::pin::Pin<Box<dyn futures::Stream<Item = AgentAction> + Send>>
        + Send
        + Sync,
>;

/// Input for progress counter tool
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProgressCounterInput {
    /// Number of steps to count
    steps: u32,
    /// Delay between steps (milliseconds)
    delay_ms: u64,
}

/// Create streaming progress counter tool
///
/// This tool demonstrates streaming by counting from 0 to N with delays,
/// emitting progress updates as chunks.
///
/// # Returns
///
/// Returns `(Tool, StreamExecutor)` tuple where:
/// - `Tool` is the tool definition for LLM
/// - `StreamExecutor` is a function that returns a Stream of `AgentAction`
#[must_use]
pub fn progress_counter_tool() -> (Tool, StreamExecutor) {
    let tool = Tool {
        name: "progress_counter".to_string(),
        description: "Counts from 0 to N with progress updates. Returns incremental progress as it counts. Useful for demonstrating streaming tools.".to_string(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "steps": {
                    "type": "integer",
                    "description": "Number of steps to count (1-100)"
                },
                "delay_ms": {
                    "type": "integer",
                    "description": "Delay between steps in milliseconds (1-1000)"
                }
            },
            "required": ["steps", "delay_ms"]
        }),
    };

    let executor = Arc::new(move |tool_use_id: String, input: String| {
        Box::pin(async_stream::stream! {
            // Parse input
            let parsed: Result<ProgressCounterInput, _> = serde_json::from_str(&input);

            let input = match parsed {
                Ok(i) => i,
                Err(e) => {
                    yield AgentAction::ToolComplete {
                        tool_use_id,
                        result: Err(ToolError {
                            message: format!("Invalid input: {e}"),
                        }),
                    };
                    return;
                }
            };

            // Validate bounds
            let steps = input.steps.min(100);
            let delay_ms = input.delay_ms.min(1000);

            // Emit progress chunks
            for i in 0..=steps {
                let progress = (i * 100) / steps;
                yield AgentAction::ToolChunk {
                    tool_use_id: tool_use_id.clone(),
                    content: format!("Progress: {progress}% (step {i}/{steps})\n"),
                };

                // Sleep between steps (except after last)
                if i < steps {
                    sleep(Duration::from_millis(delay_ms)).await;
                }
            }

            // Emit completion
            yield AgentAction::ToolComplete {
                tool_use_id,
                result: Ok(format!("Completed: {steps} steps")),
            };
        })
            as std::pin::Pin<Box<dyn futures::Stream<Item = AgentAction> + Send>>
    });

    (tool, executor)
}

/// Input for file streaming tool
#[derive(Debug, Clone, Serialize, Deserialize)]
struct StreamLinesInput {
    /// Text to stream line-by-line
    text: String,
    /// Delay between lines (milliseconds)
    delay_ms: u64,
}

/// Create streaming line-by-line tool
///
/// This tool demonstrates streaming by emitting text line-by-line with delays.
///
/// # Returns
///
/// Returns `(Tool, StreamExecutor)` tuple
#[must_use]
pub fn stream_lines_tool() -> (Tool, StreamExecutor) {
    let tool = Tool {
        name: "stream_lines".to_string(),
        description: "Streams text content line-by-line. Returns each line incrementally with configurable delay. Useful for large text processing.".to_string(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "text": {
                    "type": "string",
                    "description": "Text content to stream"
                },
                "delay_ms": {
                    "type": "integer",
                    "description": "Delay between lines in milliseconds (0-1000)"
                }
            },
            "required": ["text", "delay_ms"]
        }),
    };

    let executor = Arc::new(move |tool_use_id: String, input: String| {
        Box::pin(async_stream::stream! {
            // Parse input
            let parsed: Result<StreamLinesInput, _> = serde_json::from_str(&input);

            let input = match parsed {
                Ok(i) => i,
                Err(e) => {
                    yield AgentAction::ToolComplete {
                        tool_use_id,
                        result: Err(ToolError {
                            message: format!("Invalid input: {e}"),
                        }),
                    };
                    return;
                }
            };

            let delay_ms = input.delay_ms.min(1000);
            let lines: Vec<&str> = input.text.lines().collect();
            let total_lines = lines.len();

            // Emit each line as a chunk
            for (i, line) in lines.iter().enumerate() {
                yield AgentAction::ToolChunk {
                    tool_use_id: tool_use_id.clone(),
                    content: format!("{line}\n"),
                };

                // Sleep between lines (except after last)
                if i < total_lines - 1 && delay_ms > 0 {
                    sleep(Duration::from_millis(delay_ms)).await;
                }
            }

            // Emit completion
            yield AgentAction::ToolComplete {
                tool_use_id,
                result: Ok(format!("Streamed {total_lines} lines")),
            };
        })
            as std::pin::Pin<Box<dyn futures::Stream<Item = AgentAction> + Send>>
    });

    (tool, executor)
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;

    #[tokio::test]
    async fn test_progress_counter_tool() {
        let (tool, executor) = progress_counter_tool();

        assert_eq!(tool.name, "progress_counter");

        let input = json!({
            "steps": 3,
            "delay_ms": 10
        })
        .to_string();

        let mut stream = executor("test_1".to_string(), input);
        let mut chunks = Vec::new();
        let mut complete = None;

        while let Some(action) = stream.next().await {
            match action {
                AgentAction::ToolChunk { content, .. } => {
                    chunks.push(content);
                }
                AgentAction::ToolComplete { result, .. } => {
                    complete = Some(result);
                }
                _ => {}
            }
        }

        // Should have 4 chunks (0%, 33%, 66%, 100%)
        assert_eq!(chunks.len(), 4);
        assert!(chunks[0].contains("Progress: 0%"));
        assert!(chunks[3].contains("Progress: 100%"));

        // Should complete successfully
        assert!(complete.is_some());
        assert!(complete.as_ref().is_some_and(|r| r.as_ref().is_ok_and(|s| s.contains("Completed: 3 steps"))));
    }

    #[tokio::test]
    async fn test_progress_counter_invalid_input() {
        let (_tool, executor) = progress_counter_tool();

        let input = "invalid json".to_string();
        let mut stream = executor("test_2".to_string(), input);

        let action = stream.next().await;
        assert!(matches!(action, Some(AgentAction::ToolComplete { result: Err(_), .. })));
    }

    #[tokio::test]
    async fn test_stream_lines_tool() {
        let (tool, executor) = stream_lines_tool();

        assert_eq!(tool.name, "stream_lines");

        let input = json!({
            "text": "Line 1\nLine 2\nLine 3",
            "delay_ms": 0
        })
        .to_string();

        let mut stream = executor("test_3".to_string(), input);
        let mut chunks = Vec::new();
        let mut complete = None;

        while let Some(action) = stream.next().await {
            match action {
                AgentAction::ToolChunk { content, .. } => {
                    chunks.push(content);
                }
                AgentAction::ToolComplete { result, .. } => {
                    complete = Some(result);
                }
                _ => {}
            }
        }

        // Should have 3 chunks
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0], "Line 1\n");
        assert_eq!(chunks[1], "Line 2\n");
        assert_eq!(chunks[2], "Line 3\n");

        // Should complete successfully
        assert!(complete.is_some());
        assert!(complete.as_ref().is_some_and(|r| r.as_ref().is_ok_and(|s| s.contains("Streamed 3 lines"))));
    }

    #[tokio::test]
    async fn test_stream_lines_empty() {
        let (_tool, executor) = stream_lines_tool();

        let input = json!({
            "text": "",
            "delay_ms": 0
        })
        .to_string();

        let mut stream = executor("test_4".to_string(), input);
        let mut complete = None;

        while let Some(action) = stream.next().await {
            if let AgentAction::ToolComplete { result, .. } = action {
                complete = Some(result);
            }
        }

        // Should complete with 0 lines (empty string has no lines)
        assert!(complete.is_some());
        assert!(complete.as_ref().is_some_and(|r| r.as_ref().is_ok_and(|s| s.contains("Streamed 0 lines") || s.contains("Streamed 1 lines"))));
    }
}
