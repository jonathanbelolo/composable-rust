//! Todo management tools for agents
//!
//! Provides in-memory todo list management (not persisted across agent restarts).
//! For persistence, use with a database tool.

use composable_rust_core::agent::{Tool, ToolError, ToolExecutorFn, ToolResult};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// Todo item with ID, title, and completion status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoItem {
    /// Unique ID
    pub id: u64,
    /// Todo title/description
    pub title: String,
    /// Whether the todo is completed
    pub completed: bool,
}

/// In-memory todo store (shared across tool calls)
#[derive(Debug, Clone)]
pub struct TodoStore {
    todos: Arc<RwLock<HashMap<u64, TodoItem>>>,
    next_id: Arc<RwLock<u64>>,
}

impl TodoStore {
    /// Create a new empty todo store
    #[must_use]
    pub fn new() -> Self {
        Self {
            todos: Arc::new(RwLock::new(HashMap::new())),
            next_id: Arc::new(RwLock::new(1)),
        }
    }
}

impl Default for TodoStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Create the `todo_add` tool
///
/// Add a new todo item.
///
/// Returns JSON:
/// ```json
/// {
///   "id": 1,
///   "title": "Buy groceries",
///   "completed": false
/// }
/// ```
#[must_use]
pub fn todo_add_tool(store: TodoStore) -> (Tool, ToolExecutorFn) {
    let tool = Tool {
        name: "todo_add".to_string(),
        description: "Add a new todo item".to_string(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "title": {
                    "type": "string",
                    "description": "Todo title/description"
                }
            },
            "required": ["title"]
        }),
    };

    let executor = Arc::new(move |input: String| {
        let store = store.clone();
        Box::pin(async move {
            let parsed: serde_json::Value = serde_json::from_str(&input).map_err(|e| {
                ToolError {
                    message: format!("Invalid input JSON: {e}"),
                }
            })?;

            let title = parsed["title"]
                .as_str()
                .ok_or_else(|| ToolError {
                    message: "Missing 'title' field".to_string(),
                })?
                .to_string();

            // Get next ID
            let id = {
                let mut next_id = store.next_id.write().expect(
                    "Todo next_id lock poisoned - indicates a panic in another thread",
                );
                let id = *next_id;
                *next_id += 1;
                id
            };

            // Create todo
            let todo = TodoItem {
                id,
                title,
                completed: false,
            };

            // Insert into store
            {
                let mut todos = store
                    .todos
                    .write()
                    .expect("Todo store lock poisoned - indicates a panic in another thread");
                todos.insert(id, todo.clone());
            }

            let output = json!(todo);
            Ok(output.to_string())
        }) as std::pin::Pin<
            Box<dyn std::future::Future<Output = ToolResult> + Send>,
        >
    }) as ToolExecutorFn;

    (tool, executor)
}

/// Create the `todo_list` tool
///
/// List all todo items.
///
/// Returns JSON:
/// ```json
/// {
///   "todos": [
///     {"id": 1, "title": "Buy groceries", "completed": false},
///     {"id": 2, "title": "Write code", "completed": true}
///   ]
/// }
/// ```
#[must_use]
pub fn todo_list_tool(store: TodoStore) -> (Tool, ToolExecutorFn) {
    let tool = Tool {
        name: "todo_list".to_string(),
        description: "List all todo items".to_string(),
        input_schema: json!({
            "type": "object",
            "properties": {}
        }),
    };

    let executor = Arc::new(move |_input: String| {
        let store = store.clone();
        Box::pin(async move {
            let todos = {
                let todos_lock = store
                    .todos
                    .read()
                    .expect("Todo store lock poisoned - indicates a panic in another thread");
                let mut todos: Vec<TodoItem> = todos_lock.values().cloned().collect();
                todos.sort_by_key(|t| t.id);
                todos
            };

            let output = json!({
                "todos": todos
            });

            Ok(output.to_string())
        }) as std::pin::Pin<
            Box<dyn std::future::Future<Output = ToolResult> + Send>,
        >
    }) as ToolExecutorFn;

    (tool, executor)
}

/// Create the `todo_complete` tool
///
/// Mark a todo as completed.
///
/// Returns JSON:
/// ```json
/// {
///   "id": 1,
///   "title": "Buy groceries",
///   "completed": true
/// }
/// ```
#[must_use]
pub fn todo_complete_tool(store: TodoStore) -> (Tool, ToolExecutorFn) {
    let tool = Tool {
        name: "todo_complete".to_string(),
        description: "Mark a todo as completed".to_string(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "number",
                    "description": "Todo ID to mark as completed"
                }
            },
            "required": ["id"]
        }),
    };

    let executor = Arc::new(move |input: String| {
        let store = store.clone();
        Box::pin(async move {
            let parsed: serde_json::Value = serde_json::from_str(&input).map_err(|e| {
                ToolError {
                    message: format!("Invalid input JSON: {e}"),
                }
            })?;

            let id = parsed["id"].as_u64().ok_or_else(|| ToolError {
                message: "Missing or invalid 'id' field".to_string(),
            })?;

            // Update todo
            let todo = {
                let mut todos = store
                    .todos
                    .write()
                    .expect("Todo store lock poisoned - indicates a panic in another thread");

                let todo = todos.get_mut(&id).ok_or_else(|| ToolError {
                    message: format!("Todo not found: {id}"),
                })?;

                todo.completed = true;
                todo.clone()
            };

            let output = json!(todo);
            Ok(output.to_string())
        }) as std::pin::Pin<
            Box<dyn std::future::Future<Output = ToolResult> + Send>,
        >
    }) as ToolExecutorFn;

    (tool, executor)
}

/// Create the `todo_delete` tool
///
/// Delete a todo item.
///
/// Returns JSON:
/// ```json
/// {
///   "id": 1,
///   "deleted": true
/// }
/// ```
#[must_use]
pub fn todo_delete_tool(store: TodoStore) -> (Tool, ToolExecutorFn) {
    let tool = Tool {
        name: "todo_delete".to_string(),
        description: "Delete a todo item".to_string(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "number",
                    "description": "Todo ID to delete"
                }
            },
            "required": ["id"]
        }),
    };

    let executor = Arc::new(move |input: String| {
        let store = store.clone();
        Box::pin(async move {
            let parsed: serde_json::Value = serde_json::from_str(&input).map_err(|e| {
                ToolError {
                    message: format!("Invalid input JSON: {e}"),
                }
            })?;

            let id = parsed["id"].as_u64().ok_or_else(|| ToolError {
                message: "Missing or invalid 'id' field".to_string(),
            })?;

            // Delete todo
            {
                let mut todos = store
                    .todos
                    .write()
                    .expect("Todo store lock poisoned - indicates a panic in another thread");

                if todos.remove(&id).is_none() {
                    return Err(ToolError {
                        message: format!("Todo not found: {id}"),
                    });
                }
            }

            let output = json!({
                "id": id,
                "deleted": true
            });

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
    fn test_todo_add_tool_schema() {
        let store = TodoStore::new();
        let (tool, _executor) = todo_add_tool(store);
        assert_eq!(tool.name, "todo_add");
        assert!(tool.input_schema.is_object());
    }

    #[test]
    fn test_todo_list_tool_schema() {
        let store = TodoStore::new();
        let (tool, _executor) = todo_list_tool(store);
        assert_eq!(tool.name, "todo_list");
        assert!(tool.input_schema.is_object());
    }

    #[test]
    fn test_todo_complete_tool_schema() {
        let store = TodoStore::new();
        let (tool, _executor) = todo_complete_tool(store);
        assert_eq!(tool.name, "todo_complete");
        assert!(tool.input_schema.is_object());
    }

    #[test]
    fn test_todo_delete_tool_schema() {
        let store = TodoStore::new();
        let (tool, _executor) = todo_delete_tool(store);
        assert_eq!(tool.name, "todo_delete");
        assert!(tool.input_schema.is_object());
    }

    #[tokio::test]
    async fn test_todo_workflow() {
        let store = TodoStore::new();

        // Add a todo
        let (_add_tool, add_executor) = todo_add_tool(store.clone());
        let add_result = add_executor(
            json!({
                "title": "Test todo"
            })
            .to_string(),
        )
        .await;
        assert!(add_result.is_ok());

        let added: serde_json::Value = serde_json::from_str(&add_result.expect("should succeed")).expect("valid JSON");
        let todo_id = added["id"].as_u64().expect("should have id");
        assert_eq!(added["title"], "Test todo");
        assert_eq!(added["completed"], false);

        // List todos
        let (_list_tool, list_executor) = todo_list_tool(store.clone());
        let list_result = list_executor(json!({}).to_string()).await;
        assert!(list_result.is_ok());

        let list: serde_json::Value = serde_json::from_str(&list_result.expect("should succeed")).expect("valid JSON");
        assert_eq!(list["todos"].as_array().expect("should be array").len(), 1);

        // Complete todo
        let (_complete_tool, complete_executor) = todo_complete_tool(store.clone());
        let complete_result = complete_executor(
            json!({
                "id": todo_id
            })
            .to_string(),
        )
        .await;
        assert!(complete_result.is_ok());

        let completed: serde_json::Value = serde_json::from_str(&complete_result.expect("should succeed")).expect("valid JSON");
        assert_eq!(completed["completed"], true);

        // Delete todo
        let (_delete_tool, delete_executor) = todo_delete_tool(store.clone());
        let delete_result = delete_executor(
            json!({
                "id": todo_id
            })
            .to_string(),
        )
        .await;
        assert!(delete_result.is_ok());

        // List should be empty
        let list_result2 = list_executor(json!({}).to_string()).await;
        assert!(list_result2.is_ok());

        let list2: serde_json::Value = serde_json::from_str(&list_result2.expect("should succeed")).expect("valid JSON");
        assert_eq!(list2["todos"].as_array().expect("should be array").len(), 0);
    }
}
