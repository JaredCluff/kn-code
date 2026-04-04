use crate::traits::*;
use async_trait::async_trait;

#[derive(Debug)]
pub struct AgentTool;

impl Default for AgentTool {
    fn default() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for AgentTool {
    fn name(&self) -> &str {
        "Agent"
    }
    fn description(&self) -> &str {
        "Spawn a sub-agent"
    }
    fn prompt(&self) -> &str {
        "Use this to delegate tasks to sub-agents."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "description": { "type": "string", "description": "What this agent will do" },
                "prompt": { "type": "string", "description": "Instructions for the agent" },
                "model": { "type": "string" },
                "run_in_background": { "type": "boolean" }
            },
            "required": ["description", "prompt"]
        })
    }

    fn is_destructive(&self) -> bool {
        true
    }

    async fn call(
        &self,
        _input: serde_json::Value,
        _context: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        Err(ToolError::ExecutionFailed(
            "Sub-agent delegation is not yet implemented".to_string(),
        ))
    }
}
