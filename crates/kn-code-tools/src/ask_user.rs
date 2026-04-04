use crate::traits::*;
use async_trait::async_trait;

#[derive(Debug)]
pub struct AskUserTool;

impl Default for AskUserTool {
    fn default() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for AskUserTool {
    fn name(&self) -> &str {
        "AskUser"
    }
    fn description(&self) -> &str {
        "Ask the user a question"
    }
    fn prompt(&self) -> &str {
        "Use this to ask the user questions."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "question": { "type": "string" },
                "choices": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "label": { "type": "string" },
                            "description": { "type": "string" }
                        }
                    }
                }
            },
            "required": ["question"]
        })
    }

    async fn call(
        &self,
        _input: serde_json::Value,
        context: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        if context.is_headless {
            return Err(ToolError::ExecutionFailed(
                "Cannot ask user in headless mode".to_string(),
            ));
        }
        Err(ToolError::ExecutionFailed(
            "Interactive user prompts are not yet implemented".to_string(),
        ))
    }
}
