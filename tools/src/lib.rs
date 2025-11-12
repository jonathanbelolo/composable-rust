//! Built-in tools for AI agents (Phase 8.2)
//!
//! This crate provides a comprehensive set of built-in tools for agent systems,
//! including HTTP requests, file I/O, time, calculations, data manipulation,
//! todo management, and mock search tools.
//!
//! ## Design Principles
//!
//! **LLM-Agnostic**: All tools return structured, standard data formats:
//! - JSON with type/format/data fields
//! - Standard formats (base64 for binary, ISO8601 for dates)
//! - Rich metadata (file size, MIME type, dimensions)
//!
//! Tools do NOT:
//! - Assume specific LLM (Claude, GPT, Gemini, etc.)
//! - Format data for specific API
//! - Reference LLM capabilities in descriptions
//!
//! **Separation of Concerns**:
//! - **Tools** (this crate): Return raw/structured data
//! - **Agent Environments**: Transform data for specific LLM APIs
//! - **LLM Client Crates**: Handle LLM-specific protocols
//!
//! ## Modules
//!
//! - `http`: HTTP request tools (request, get, `get_markdown`)
//! - `file_io`: File I/O tools (`read_file`, `list_directory`)
//! - `time`: Time tools (`current_time`)
//! - `calculation`: Calculation tools (calculate)
//! - `data`: Data manipulation tools (`json_query`, `string_transform`)
//! - `todo`: Todo management tools (add, list, complete, delete)
//! - `mock`: Mock tools for testing (`memory_search`, `web_search`)
//! - `streaming`: Streaming tool examples (`progress_counter`, `stream_lines`)
//! - `registry`: Tool registry for dynamic tool management
//! - `retry`: Retry policies and timeout handling

pub mod calculation;
pub mod data;
pub mod file_io;
pub mod http;
pub mod mock;
pub mod registry;
pub mod retry;
pub mod streaming;
pub mod time;
pub mod todo;

pub use composable_rust_core::agent::{Tool, ToolExecutorFn, ToolResult};

// Re-export commonly used types
pub use registry::ToolRegistry;
pub use retry::{execute_with_retry, RetryPolicy, ToolConfig};
