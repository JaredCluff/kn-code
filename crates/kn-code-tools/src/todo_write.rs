use crate::traits::*;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TodoItem {
    content: String,
    status: String,
}

#[derive(Debug)]
pub struct TodoWriteTool {
    todos: Arc<Mutex<Vec<TodoItem>>>,
}

impl Default for TodoWriteTool {
    fn default() -> Self {
        Self {
            todos: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

#[derive(Debug, Deserialize)]
struct TodoInput {
    todos: Vec<TodoItem>,
}

#[async_trait]
impl Tool for TodoWriteTool {
    fn name(&self) -> &str {
        "TodoWrite"
    }
    fn description(&self) -> &str {
        "Manage a todo list for the current session"
    }
    fn prompt(&self) -> &str {
        "Use this to manage todos. Replaces the entire list on each call."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "todos": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "content": { "type": "string" },
                            "status": { "type": "string", "enum": ["pending", "in_progress", "completed"] }
                        },
                        "required": ["content", "status"]
                    }
                }
            },
            "required": ["todos"]
        })
    }

    async fn call(
        &self,
        input: serde_json::Value,
        _context: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let parsed: TodoInput =
            serde_json::from_value(input.clone()).map_err(|e| ToolError::ValidationFailed {
                message: e.to_string(),
            })?;

        let mut todos = self.todos.lock().await;
        *todos = parsed.todos;

        let summary: Vec<String> = todos
            .iter()
            .map(|t| format!("[{}] {}", t.status, t.content))
            .collect();

        let todo_count = todos.len();
        let pending = todos.iter().filter(|t| t.status == "pending").count();
        let in_progress = todos.iter().filter(|t| t.status == "in_progress").count();
        let completed = todos.iter().filter(|t| t.status == "completed").count();
        let todos_snapshot: Vec<_> = todos.clone();
        drop(todos);

        Ok(ToolResult {
            content: ToolContent::Text(format!(
                "Todo list updated ({} items):\n{}",
                todo_count,
                summary.join("\n")
            )),
            new_messages: Vec::new(),
            persisted: false,
            persisted_path: None,
            structured_content: Some(serde_json::json!({
                "todos": todos_snapshot,
                "total": todo_count,
                "pending": pending,
                "in_progress": in_progress,
                "completed": completed,
            })),
        })
    }
}
