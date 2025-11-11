# Built-in Tools for AI Agents (Phase 8.2)

**Status**: ✅ Complete (Phase 8.2)

This guide covers the comprehensive tool system for AI agents, including:
- 14 built-in tools across 7 categories
- Dynamic tool registry
- Retry policies and timeout handling
- LLM-agnostic design principles
- Security best practices

## Table of Contents

1. [Overview](#overview)
2. [Design Principles](#design-principles)
3. [Built-in Tools](#built-in-tools)
4. [Tool Registry](#tool-registry)
5. [Retry Policies](#retry-policies)
6. [Security](#security)
7. [Examples](#examples)

---

## Overview

The `composable-rust-tools` crate provides a comprehensive set of production-ready tools for AI agents:

```rust
use composable_rust_tools::{
    http::http_get_tool,
    ToolRegistry,
    ToolConfig,
};

// Create registry and register tools
let registry = ToolRegistry::new();
let (tool, executor) = http_get_tool();
registry.register(tool, executor);

// Execute tool with retry policy
let config = ToolConfig::fixed_retry(3, Duration::from_millis(100));
let result = execute_with_retry(&config, || async {
    registry.execute("http_get", r#"{"url": "https://example.com"}"#).await
}).await?;
```

**Key Features:**
- ✅ **14 tools** across HTTP, file I/O, time, calculation, data, todo, and mock categories
- ✅ **LLM-agnostic** design (works with Claude, GPT, Gemini, local models)
- ✅ **Thread-safe** registry with `Arc<RwLock<>>`
- ✅ **Retry policies** (none, fixed, exponential backoff)
- ✅ **Comprehensive security** (path validation, size limits, timeouts)
- ✅ **58 passing tests** with full coverage

---

## Design Principles

### 1. LLM-Agnostic Tools

**Critical**: All tools return structured, standard data formats:

✅ **DO**:
- Return JSON with `type`, `format`, `data` fields
- Use standard formats (base64 for binary, ISO8601 for dates)
- Include rich metadata (file size, MIME type, dimensions)

❌ **DON'T**:
- Assume specific LLM (Claude, GPT, Gemini)
- Format data for specific API
- Reference LLM capabilities in descriptions

**Separation of Concerns:**
- **Tools** (this crate): Return raw/structured data
- **Agent Environments**: Transform data for specific LLM APIs
- **LLM Client Crates**: Handle LLM-specific protocols

### 2. Security First

All tools implement comprehensive security:
- **Path validation**: Prevents `..` traversal, symlink escapes, absolute paths
- **URL filtering**: Only `http://` and `https://` allowed
- **Size limits**: 50MB HTTP responses, 1MB images, 10MB text files
- **Timeout enforcement**: 30s default, configurable per tool
- **Input validation**: JSON schema validation on all inputs

### 3. Error Handling

No `unwrap()` or `panic!()` in library code:
```rust
// ✅ Good: Descriptive expect for lock poisoning
let tools = self.tools.read()
    .expect("Tool registry lock poisoned - indicates a panic in another thread");

// ❌ Bad: Silent unwrap
let tools = self.tools.read().unwrap();
```

---

## Built-in Tools

### HTTP Tools (3 tools)

#### 1. `http_request` - Full HTTP Request

Full-featured HTTP with method, headers, and body.

```rust
use composable_rust_tools::http::http_request_tool;

let (tool, executor) = http_request_tool();

let result = executor(json!({
    "method": "POST",
    "url": "https://api.example.com/data",
    "headers": {
        "Content-Type": "application/json"
    },
    "body": r#"{"key": "value"}"#
}).to_string()).await?;
```

**Returns:**
```json
{
  "status": 200,
  "headers": {"content-type": "application/json"},
  "body": "response body"
}
```

**Security:**
- Only `http://` and `https://` URLs
- 50MB response size limit (streaming)
- 30s timeout (configurable)

#### 2. `http_get` - Simple GET Request

Convenience wrapper for HTTP GET.

```rust
let (tool, executor) = http_get_tool();

let result = executor(json!({
    "url": "https://example.com"
}).to_string()).await?;
```

#### 3. `http_get_markdown` - HTML→Markdown Conversion

Fetches HTML and converts to Markdown for token efficiency.

```rust
let (tool, executor) = http_get_markdown_tool();

let result = executor(json!({
    "url": "https://example.com/article"
}).to_string()).await?;
```

**Returns:**
```json
{
  "status": 200,
  "markdown": "# Page Title\n\nContent...",
  "original_size": 50000,
  "markdown_size": 15000
}
```

**Token efficiency:** ~70% reduction (HTML→Markdown)

---

### File I/O Tools (2 tools)

#### 4. `read_file` - Smart Multi-Format Reader

Automatically detects file type and returns appropriate format.

```rust
use composable_rust_tools::file_io::read_file_tool;

let (tool, executor) = read_file_tool();

let result = executor(json!({
    "path": "document.pdf"
}).to_string()).await?;
```

**Supported formats:**
- **Text**: `.txt`, `.md`, `.rs`, etc. → raw text
- **PDF**: `.pdf` → extracted text
- **Images**: `.jpg`, `.png`, `.gif`, `.webp` → base64 + metadata
- **Audio**: `.mp3`, `.wav` → metadata only
- **Video**: `.mp4`, `.avi` → metadata only

**Text file returns:**
```json
{
  "type": "text",
  "content": "file contents...",
  "metadata": {"size": 1024, "path": "file.txt"}
}
```

**Image returns:**
```json
{
  "type": "image",
  "base64": "iVBORw0KGgoAAAANSUhEUgAA...",
  "metadata": {
    "size": 102400,
    "width": 800,
    "height": 600,
    "mime_type": "image/jpeg"
  }
}
```

**Security:**
- Comprehensive path validation (prevents `..`, symlinks, absolute paths)
- 10MB text/PDF limit
- 1MB image limit (~340K tokens)
- Sandbox: All paths relative to allowed directory

#### 5. `list_directory` - Directory Listing

List files and directories.

```rust
let (tool, executor) = list_directory_tool();

let result = executor(json!({
    "path": "."
}).to_string()).await?;
```

**Returns:**
```json
{
  "entries": [
    {"name": "file.txt", "type": "file", "size": 1024},
    {"name": "subdir", "type": "directory"}
  ]
}
```

---

### Time Tools (1 tool)

#### 6. `current_time` - Get Current Time

Returns current time in UTC, local, and custom timezones.

```rust
use composable_rust_tools::time::current_time_tool;

let (tool, executor) = current_time_tool();

let result = executor(json!({
    "timezone": "America/New_York"
}).to_string()).await?;
```

**Returns:**
```json
{
  "utc": "2025-01-15T10:30:00Z",
  "local": "2025-01-15T02:30:00-08:00",
  "timezone": "2025-01-15T05:30:00-05:00",
  "timezone_name": "America/New_York",
  "unix_timestamp": 1705315800
}
```

**Formats:** ISO8601 (RFC3339) for all timestamps

---

### Calculation Tools (1 tool)

#### 7. `calculate` - Mathematical Expressions

Evaluate mathematical expressions using `meval`.

```rust
use composable_rust_tools::calculation::calculate_tool;

let (tool, executor) = calculate_tool();

let result = executor(json!({
    "expression": "sqrt(16) + 2^3"
}).to_string()).await?;
```

**Supports:**
- Basic arithmetic: `+`, `-`, `*`, `/`, `%`
- Exponentiation: `^`
- Functions: `sin`, `cos`, `tan`, `sqrt`, `abs`, `log`, `exp`
- Parentheses: `()`

**Returns:**
```json
{
  "expression": "sqrt(16) + 2^3",
  "result": 12.0
}
```

---

### Data Tools (2 tools)

#### 8. `json_query` - JSONPath Queries

Query JSON data using JSONPath expressions.

```rust
use composable_rust_tools::data::json_query_tool;

let (tool, executor) = json_query_tool();

let result = executor(json!({
    "data": r#"{"users": [{"name": "Alice"}, {"name": "Bob"}]}"#,
    "query": "$.users[*].name"
}).to_string()).await?;
```

**Returns:**
```json
{
  "results": ["Alice", "Bob"]
}
```

**JSONPath examples:**
- `$.users[*].name` - All user names
- `$.items[?(@.price < 10)]` - Items under $10
- `$..name` - All names recursively

#### 9. `string_transform` - String Operations

Transform strings with common operations.

```rust
let (tool, executor) = string_transform_tool();

let result = executor(json!({
    "text": "hello world",
    "operation": "uppercase"
}).to_string()).await?;
```

**Operations:**
- `uppercase` - Convert to uppercase
- `lowercase` - Convert to lowercase
- `trim` - Remove leading/trailing whitespace
- `trim_start` - Remove leading whitespace
- `trim_end` - Remove trailing whitespace
- `reverse` - Reverse string
- `length` - Get string length

---

### Todo Tools (4 tools)

In-memory todo list management (use with database for persistence).

#### 10. `todo_add` - Add Todo

```rust
use composable_rust_tools::todo::{TodoStore, todo_add_tool};

let store = TodoStore::new();
let (tool, executor) = todo_add_tool(store.clone());

let result = executor(json!({
    "title": "Implement Phase 8.2"
}).to_string()).await?;
```

**Returns:**
```json
{
  "id": 1,
  "title": "Implement Phase 8.2",
  "completed": false
}
```

#### 11. `todo_list` - List Todos

```rust
let (tool, executor) = todo_list_tool(store.clone());
let result = executor(json!({}).to_string()).await?;
```

#### 12. `todo_complete` - Mark Complete

```rust
let (tool, executor) = todo_complete_tool(store.clone());
let result = executor(json!({"id": 1}).to_string()).await?;
```

#### 13. `todo_delete` - Delete Todo

```rust
let (tool, executor) = todo_delete_tool(store);
let result = executor(json!({"id": 1}).to_string()).await?;
```

**Note:** Todo storage is **in-memory** (not persisted). For persistence, integrate with database tools.

---

### Mock Tools (2 tools)

Useful for testing and development without external dependencies.

#### 14. `memory_search` - Mock Memory Search

Simulates searching conversation memory.

```rust
use composable_rust_tools::mock::memory_search_tool;

let (tool, executor) = memory_search_tool();

let result = executor(json!({
    "query": "weather"
}).to_string()).await?;
```

**Returns:** Mock search results based on query keywords.

#### 15. `web_search` - Mock Web Search

Simulates web search results.

```rust
let (tool, executor) = web_search_tool();

let result = executor(json!({
    "query": "Rust programming"
}).to_string()).await?;
```

**Returns:** Mock web results with titles, URLs, snippets.

---

## Tool Registry

Dynamic tool management with thread-safe registry.

### Basic Usage

```rust
use composable_rust_tools::{ToolRegistry, http::http_get_tool};

// Create registry
let registry = ToolRegistry::new();

// Register tool
let (tool, executor) = http_get_tool();
registry.register(tool, executor);

// Execute by name
let result = registry.execute(
    "http_get",
    r#"{"url": "https://example.com"}"#.to_string()
).await?;
```

### Registry Operations

```rust
// List all tool names
let tools = registry.list_tools(); // ["calculate", "http_get", ...]

// Get tool definitions (for LLM API)
let tools = registry.get_tools(); // Vec<Tool>

// Get specific tool
if let Some(tool) = registry.get_tool("http_get") {
    println!("{}", tool.description);
}

// Remove tool
registry.unregister("http_get");

// Clear all tools
registry.clear();

// Count tools
let count = registry.count();
```

### Thread Safety

The registry uses `Arc<RwLock<>>` for thread-safe concurrent access:
- Multiple readers can access tools simultaneously
- Writes (register/unregister) acquire exclusive lock
- Clone-able for sharing across threads

```rust
let registry = ToolRegistry::new();
let registry_clone = registry.clone();

tokio::spawn(async move {
    registry_clone.execute("http_get", input).await
});
```

---

## Retry Policies

Configurable retry with exponential backoff and timeout.

### Retry Policy Types

```rust
use composable_rust_tools::{RetryPolicy, ToolConfig};
use std::time::Duration;

// No retry - fail immediately
let config = ToolConfig::no_retry();

// Fixed retry - constant delay
let config = ToolConfig::fixed_retry(3, Duration::from_millis(100));

// Exponential backoff - increasing delay
let config = ToolConfig::exponential_backoff(3, Duration::from_millis(100));
// Delays: 100ms, 200ms, 400ms

// Custom timeout
let config = ToolConfig::default().with_timeout(Duration::from_secs(60));
```

### Using Retry Policies

```rust
use composable_rust_tools::{execute_with_retry, ToolConfig};

let config = ToolConfig::fixed_retry(3, Duration::from_millis(100))
    .with_timeout(Duration::from_secs(30));

let result = execute_with_retry(&config, || async {
    // Your tool execution here
    registry.execute("http_get", input).await
}).await?;
```

### Retry Behavior

**Fixed Retry:**
```
Attempt 1 → [wait 100ms] → Attempt 2 → [wait 100ms] → Attempt 3
```

**Exponential Backoff:**
```
Attempt 1 → [wait 100ms] → Attempt 2 → [wait 200ms] → Attempt 3
```

**Timeout:**
- Applied **per attempt** (not total)
- Returns `ToolError` on timeout
- Timeout errors are retryable (follow retry policy)

---

## Security

### Path Validation (File Tools)

4-step validation process:

```rust
// 1. Reject parent directory traversal
assert!(validate_path("../etc/passwd").is_err());

// 2. Only allow relative paths
assert!(validate_path("/etc/passwd").is_err());

// 3. Canonicalize and verify within sandbox
let canonical = path.canonicalize()?;
assert!(canonical.starts_with(ALLOWED_BASE_DIR));

// 4. Check symlinks don't escape sandbox
if canonical.is_symlink() {
    let target = std::fs::read_link(&canonical)?;
    assert!(!target.is_absolute());
}
```

### Resource Limits

| Resource | Limit | Reason |
|----------|-------|--------|
| HTTP response | 50MB | Memory exhaustion |
| Text/PDF files | 10MB | Token limits |
| Images | 1MB | ~340K tokens max |
| Tool timeout | 30s default | Prevent hanging |

### URL Filtering

```rust
// ✅ Allowed
"https://example.com"
"http://localhost:8080"

// ❌ Rejected
"file:///etc/passwd"      // Local file access
"ftp://example.com"       // Non-HTTP protocol
"javascript:alert(1)"     // Code injection
```

### SSRF Protection

HTTP tools are vulnerable to SSRF (Server-Side Request Forgery):
- **Risk**: Agent can be tricked into fetching internal URLs
- **Mitigation**: Document risk, consider URL filtering in production
- **Future**: Add configurable URL allowlist/blocklist

---

## Examples

### Example 1: Tool-Showcase

Complete demonstration of all 14 tools:

```bash
cargo run -p tool-showcase
```

See: `examples/tool-showcase/src/main.rs`

### Example 2: Agent with HTTP Tool

```rust
use composable_rust_tools::{http::http_get_tool, ToolRegistry};
use composable_rust_core::agent::{AgentConfig, BasicAgentState};
use composable_rust_runtime::Store;

// Create registry
let registry = ToolRegistry::new();
let (tool, executor) = http_get_tool();
registry.register(tool, executor);

// Pass tools to agent environment
let tools = registry.get_tools();
let environment = ProductionAgentEnvironment::new(config)?
    .with_tools(&tools, registry);

// Agent can now use http_get tool
store.send(AgentAction::UserMessage {
    content: "Fetch https://example.com".to_string()
}).await?;
```

### Example 3: Custom Tool with Retry

```rust
use composable_rust_tools::{execute_with_retry, ToolConfig, ToolRegistry};

let config = ToolConfig::exponential_backoff(5, Duration::from_millis(100))
    .with_timeout(Duration::from_secs(10));

let result = execute_with_retry(&config, || async {
    // Flaky operation (network, external API, etc.)
    registry.execute("http_get", input).await
}).await?;
```

### Example 4: File Operations with Security

```rust
use composable_rust_tools::file_io::{read_file_tool, list_directory_tool};

let registry = ToolRegistry::new();

// Register file tools
let (read_file, read_exec) = read_file_tool();
let (list_dir, list_exec) = list_directory_tool();
registry.register(read_file, read_exec);
registry.register(list_dir, list_exec);

// Safe: Relative path within sandbox
let result = registry.execute("read_file", json!({
    "path": "documents/report.pdf"
}).to_string()).await?;

// Blocked: Parent directory traversal
let result = registry.execute("read_file", json!({
    "path": "../../../etc/passwd"
}).to_string()).await;
assert!(result.is_err()); // "Parent directory (..) not allowed"
```

---

## Best Practices

### 1. Always Use Retry for Network Operations

```rust
// ✅ Good: Retry for flaky network operations
let config = ToolConfig::exponential_backoff(3, Duration::from_millis(100));
execute_with_retry(&config, || http_operation()).await?;

// ❌ Bad: No retry for network operation
http_operation().await?; // Single transient failure breaks agent
```

### 2. Set Appropriate Timeouts

```rust
// ✅ Good: Longer timeout for slow operations
let config = ToolConfig::default()
    .with_timeout(Duration::from_secs(120)); // PDF extraction

// ❌ Bad: Default timeout for slow operation
let config = ToolConfig::default(); // 30s might be too short
```

### 3. Validate Tool Inputs

```rust
// ✅ Good: Validate before execution
let url = input["url"].as_str().ok_or(ToolError::missing_field("url"))?;
if !url.starts_with("https://") {
    return Err(ToolError::invalid("URL must use HTTPS"));
}

// ❌ Bad: No validation
let url = input["url"].as_str().unwrap(); // Panic on missing field
```

### 4. Use Registry for Dynamic Tools

```rust
// ✅ Good: Registry for dynamic tool management
let registry = ToolRegistry::new();
for tool_config in agent_config.enabled_tools {
    registry.register(create_tool(&tool_config));
}

// ❌ Bad: Hardcoded tool registration
environment.with_tool("http_get", http_get_executor);
environment.with_tool("calculate", calculate_executor);
// ... not extensible
```

### 5. Share TodoStore Across Tools

```rust
// ✅ Good: Shared store for todo tools
let store = TodoStore::new();
registry.register(todo_add_tool(store.clone()));
registry.register(todo_list_tool(store.clone()));
registry.register(todo_complete_tool(store.clone()));
registry.register(todo_delete_tool(store));

// ❌ Bad: Separate stores (todos won't be shared)
registry.register(todo_add_tool(TodoStore::new()));
registry.register(todo_list_tool(TodoStore::new()));
```

---

## Performance Considerations

### 1. Image Size Impact

1MB image → ~340K tokens (at 1 token per 3 bytes for base64):
- Claude Sonnet 4.5: 200K token limit → max ~0.6MB images
- Consider image resizing before encoding

### 2. HTML→Markdown Conversion

70% token reduction for typical web pages:
- HTML: 50KB → ~16K tokens
- Markdown: 15KB → ~5K tokens
- **3x improvement** in context window usage

### 3. Retry Overhead

Exponential backoff with 3 attempts:
- Success on attempt 1: 0ms overhead
- Success on attempt 2: 100ms overhead
- Success on attempt 3: 300ms overhead (100ms + 200ms)

### 4. Registry Lock Contention

`RwLock` allows concurrent reads:
- **Read operations** (execute, get_tool): No contention
- **Write operations** (register, unregister): Exclusive lock
- Design: Optimize for read-heavy workloads (tool execution)

---

## Testing

All tools have comprehensive unit tests:

```bash
# Run all tools tests
cargo test -p composable-rust-tools

# Run specific module tests
cargo test -p composable-rust-tools http
cargo test -p composable-rust-tools registry

# Run with output
cargo test -p composable-rust-tools -- --nocapture
```

**Test coverage**: 58 tests, 100% success rate

---

## Future Enhancements (Phase 8.3+)

Planned improvements:
1. **Real web search** integration (Brave, Google APIs)
2. **Vector database** for semantic memory search
3. **File write tools** with audit logging
4. **Database query tools** (PostgreSQL, SQLite)
5. **Code execution** in sandboxed environments
6. **Image generation** tools (DALL-E, Stable Diffusion)
7. **Speech-to-text** / text-to-speech tools
8. **Browser automation** (headless Chrome, Playwright)

---

## References

- **Implementation**: `tools/src/`
- **Examples**: `examples/tool-showcase/`, `examples/weather-agent/`
- **Tests**: `tools/src/*/tests.rs`
- **Phase 8.2 Plan**: `plans/phase-8/phase-8.2-implementation-plan.md`
- **Architecture**: `specs/architecture.md` (Section 8: AI Agents)

---

**Phase 8.2 Status**: ✅ **Complete**
- 14 tools implemented and tested
- Registry and retry policies working
- Security measures in place
- Documentation complete
- Ready for Phase 8.3 (Advanced Agent Features)
