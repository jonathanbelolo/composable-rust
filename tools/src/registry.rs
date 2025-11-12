//! Tool registry for dynamic tool management
//!
//! The registry provides:
//! - Dynamic tool registration
//! - Thread-safe tool storage
//! - Tool execution by name
//! - Tool listing and introspection

use composable_rust_core::agent::{Tool, ToolError, ToolExecutorFn, ToolResult};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// Thread-safe tool registry
///
/// The registry stores tools and their executors, allowing dynamic
/// registration and execution by name.
///
/// ## Example
///
/// ```ignore
/// use composable_rust_tools::registry::ToolRegistry;
/// use composable_rust_tools::http::http_get_tool;
///
/// let registry = ToolRegistry::new();
/// let (tool, executor) = http_get_tool();
/// registry.register(tool, executor);
///
/// // Execute tool by name
/// let result = registry.execute("http_get", r#"{"url": "https://example.com"}"#).await;
/// ```
#[derive(Clone)]
pub struct ToolRegistry {
    tools: Arc<RwLock<HashMap<String, (Tool, ToolExecutorFn)>>>,
}

impl ToolRegistry {
    /// Create a new empty tool registry
    #[must_use]
    pub fn new() -> Self {
        Self {
            tools: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Register a tool with its executor
    ///
    /// If a tool with the same name already exists, it will be replaced
    /// and this method returns `true`. Otherwise, returns `false`.
    ///
    /// ## Example
    ///
    /// ```ignore
    /// let registry = ToolRegistry::new();
    /// let (tool, executor) = http_get_tool();
    /// let replaced = registry.register(tool, executor);
    /// assert!(!replaced); // First registration
    /// ```
    ///
    /// # Panics
    ///
    /// Panics if the internal lock is poisoned (indicates a panic in another thread)
    #[allow(clippy::expect_used)]
    pub fn register(&self, tool: Tool, executor: ToolExecutorFn) -> bool {
        let mut tools = self
            .tools
            .write()
            .expect("Tool registry lock poisoned - indicates a panic in another thread");
        tools.insert(tool.name.clone(), (tool, executor)).is_some()
    }

    /// Execute a tool by name
    ///
    /// Returns the tool's result if the tool exists and executes successfully.
    /// Returns an error if:
    /// - Tool not found
    /// - Tool execution fails
    ///
    /// ## Example
    ///
    /// ```ignore
    /// let result = registry.execute("http_get", r#"{"url": "https://example.com"}"#).await;
    /// ```
    ///
    /// # Errors
    ///
    /// Returns `ToolError` if the tool is not found or execution fails
    ///
    /// # Panics
    ///
    /// Panics if the internal lock is poisoned (indicates a panic in another thread)
    #[allow(clippy::expect_used)]
    pub async fn execute(&self, name: &str, input: String) -> ToolResult {
        // Get executor (release lock quickly)
        let executor = {
            let tools = self
                .tools
                .read()
                .expect("Tool registry lock poisoned - indicates a panic in another thread");
            tools.get(name).map(|(_, executor)| executor.clone())
        };

        match executor {
            Some(executor) => executor(input).await,
            None => Err(ToolError {
                message: format!("Tool not found: {name}"),
            }),
        }
    }

    /// Get a list of all registered tool names
    ///
    /// Returns a vector of tool names sorted alphabetically.
    ///
    /// ## Example
    ///
    /// ```ignore
    /// let tools = registry.list_tools();
    /// for tool_name in tools {
    ///     println!("Registered tool: {}", tool_name);
    /// }
    /// ```
    ///
    /// # Panics
    ///
    /// Panics if the internal lock is poisoned (indicates a panic in another thread)
    #[must_use]
    #[allow(clippy::expect_used)]
    pub fn list_tools(&self) -> Vec<String> {
        let tools = self
            .tools
            .read()
            .expect("Tool registry lock poisoned - indicates a panic in another thread");
        let mut names: Vec<String> = tools.keys().cloned().collect();
        names.sort();
        names
    }

    /// Get all registered tools (for passing to LLM API)
    ///
    /// Returns a vector of `Tool` definitions sorted by name.
    ///
    /// ## Example
    ///
    /// ```ignore
    /// let tools = registry.get_tools();
    /// // Pass to Claude API: MessagesRequest { tools: Some(tools), ... }
    /// ```
    ///
    /// # Panics
    ///
    /// Panics if the internal lock is poisoned (indicates a panic in another thread)
    #[must_use]
    #[allow(clippy::expect_used)]
    pub fn get_tools(&self) -> Vec<Tool> {
        let tools = self
            .tools
            .read()
            .expect("Tool registry lock poisoned - indicates a panic in another thread");
        let mut tool_list: Vec<Tool> = tools.values().map(|(tool, _)| tool.clone()).collect();
        tool_list.sort_by(|a, b| a.name.cmp(&b.name));
        tool_list
    }

    /// Get a specific tool by name
    ///
    /// Returns `None` if the tool is not registered.
    ///
    /// ## Example
    ///
    /// ```ignore
    /// if let Some(tool) = registry.get_tool("http_get") {
    ///     println!("Tool description: {}", tool.description);
    /// }
    /// ```
    ///
    /// # Panics
    ///
    /// Panics if the internal lock is poisoned (indicates a panic in another thread)
    #[must_use]
    #[allow(clippy::expect_used)]
    pub fn get_tool(&self, name: &str) -> Option<Tool> {
        let tools = self
            .tools
            .read()
            .expect("Tool registry lock poisoned - indicates a panic in another thread");
        tools.get(name).map(|(tool, _)| tool.clone())
    }

    /// Remove a tool from the registry
    ///
    /// Returns `true` if the tool was removed, `false` if it didn't exist.
    ///
    /// ## Example
    ///
    /// ```ignore
    /// let removed = registry.unregister("http_get");
    /// assert!(removed);
    /// ```
    ///
    /// # Panics
    ///
    /// Panics if the internal lock is poisoned (indicates a panic in another thread)
    #[must_use]
    #[allow(clippy::expect_used)]
    pub fn unregister(&self, name: &str) -> bool {
        let mut tools = self
            .tools
            .write()
            .expect("Tool registry lock poisoned - indicates a panic in another thread");
        tools.remove(name).is_some()
    }

    /// Clear all tools from the registry
    ///
    /// ## Example
    ///
    /// ```ignore
    /// registry.clear();
    /// assert_eq!(registry.list_tools().len(), 0);
    /// ```
    ///
    /// # Panics
    ///
    /// Panics if the internal lock is poisoned (indicates a panic in another thread)
    #[allow(clippy::expect_used)]
    pub fn clear(&self) {
        let mut tools = self
            .tools
            .write()
            .expect("Tool registry lock poisoned - indicates a panic in another thread");
        tools.clear();
    }

    /// Get the number of registered tools
    ///
    /// ## Example
    ///
    /// ```ignore
    /// let count = registry.count();
    /// println!("Registered tools: {}", count);
    /// ```
    ///
    /// # Panics
    ///
    /// Panics if the internal lock is poisoned (indicates a panic in another thread)
    #[must_use]
    #[allow(clippy::expect_used)]
    pub fn count(&self) -> usize {
        let tools = self
            .tools
            .read()
            .expect("Tool registry lock poisoned - indicates a panic in another thread");
        tools.len()
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::{memory_search_tool, web_search_tool};
    use serde_json::json;

    #[test]
    fn test_registry_new() {
        let registry = ToolRegistry::new();
        assert_eq!(registry.count(), 0);
    }

    #[test]
    fn test_registry_register() {
        let registry = ToolRegistry::new();
        let (tool, executor) = memory_search_tool();

        let replaced = registry.register(tool, executor);
        assert!(!replaced); // First registration
        assert_eq!(registry.count(), 1);
    }

    #[test]
    fn test_registry_register_replace() {
        let registry = ToolRegistry::new();
        let (tool1, executor1) = memory_search_tool();
        let (tool2, executor2) = memory_search_tool();

        registry.register(tool1, executor1);
        let replaced = registry.register(tool2, executor2);
        assert!(replaced); // Second registration replaces
        assert_eq!(registry.count(), 1);
    }

    #[test]
    fn test_registry_list_tools() {
        let registry = ToolRegistry::new();
        let (tool1, executor1) = memory_search_tool();
        let (tool2, executor2) = web_search_tool();

        registry.register(tool1, executor1);
        registry.register(tool2, executor2);

        let tools = registry.list_tools();
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0], "memory_search"); // Sorted alphabetically
        assert_eq!(tools[1], "web_search");
    }

    #[test]
    fn test_registry_get_tools() {
        let registry = ToolRegistry::new();
        let (tool1, executor1) = memory_search_tool();
        let (tool2, executor2) = web_search_tool();

        registry.register(tool1, executor1);
        registry.register(tool2, executor2);

        let tools = registry.get_tools();
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0].name, "memory_search"); // Sorted alphabetically
        assert_eq!(tools[1].name, "web_search");
    }

    #[test]
    fn test_registry_get_tool() {
        let registry = ToolRegistry::new();
        let (tool, executor) = memory_search_tool();

        registry.register(tool, executor);

        let retrieved = registry.get_tool("memory_search");
        assert!(retrieved.is_some());
        assert_eq!(retrieved.expect("should exist").name, "memory_search");

        let not_found = registry.get_tool("nonexistent");
        assert!(not_found.is_none());
    }

    #[tokio::test]
    async fn test_registry_execute() {
        let registry = ToolRegistry::new();
        let (tool, executor) = memory_search_tool();

        registry.register(tool, executor);

        let result = registry
            .execute(
                "memory_search",
                json!({
                    "query": "weather"
                })
                .to_string(),
            )
            .await;

        assert!(result.is_ok());
        let output: serde_json::Value =
            serde_json::from_str(&result.expect("should succeed")).expect("valid JSON");
        assert_eq!(output["query"], "weather");
    }

    #[tokio::test]
    async fn test_registry_execute_not_found() {
        let registry = ToolRegistry::new();

        let result = registry
            .execute(
                "nonexistent",
                json!({
                    "query": "test"
                })
                .to_string(),
            )
            .await;

        assert!(result.is_err());
        assert!(result
            .expect_err("should fail")
            .message
            .contains("Tool not found"));
    }

    #[test]
    fn test_registry_unregister() {
        let registry = ToolRegistry::new();
        let (tool, executor) = memory_search_tool();

        registry.register(tool, executor);
        assert_eq!(registry.count(), 1);

        let removed = registry.unregister("memory_search");
        assert!(removed);
        assert_eq!(registry.count(), 0);

        let not_removed = registry.unregister("nonexistent");
        assert!(!not_removed);
    }

    #[test]
    fn test_registry_clear() {
        let registry = ToolRegistry::new();
        let (tool1, executor1) = memory_search_tool();
        let (tool2, executor2) = web_search_tool();

        registry.register(tool1, executor1);
        registry.register(tool2, executor2);
        assert_eq!(registry.count(), 2);

        registry.clear();
        assert_eq!(registry.count(), 0);
    }

    #[test]
    fn test_registry_default() {
        let registry = ToolRegistry::default();
        assert_eq!(registry.count(), 0);
    }
}
