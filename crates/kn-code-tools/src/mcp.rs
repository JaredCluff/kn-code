use crate::traits::*;
use async_trait::async_trait;

#[derive(Debug)]
pub struct McpTool;

impl Default for McpTool {
    fn default() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for McpTool {
    fn name(&self) -> &str {
        "MCP"
    }
    fn description(&self) -> &str {
        "Interact with MCP servers"
    }
    fn prompt(&self) -> &str {
        "Use this to interact with MCP servers."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "server": { "type": "string" },
                "tool": { "type": "string" },
                "input": { "type": "object" }
            },
            "required": ["server", "tool"]
        })
    }

    async fn call(
        &self,
        _input: serde_json::Value,
        _context: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        Err(ToolError::ExecutionFailed(
            "MCP server interaction is not yet implemented".to_string(),
        ))
    }
}
