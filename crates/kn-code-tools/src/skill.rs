use crate::traits::*;
use async_trait::async_trait;

#[derive(Debug)]
pub struct SkillTool;

impl Default for SkillTool {
    fn default() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for SkillTool {
    fn name(&self) -> &str {
        "Skill"
    }
    fn description(&self) -> &str {
        "Execute a skill/command"
    }
    fn prompt(&self) -> &str {
        "Use this to execute skills."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "skill": { "type": "string", "description": "Skill name" },
                "args": { "type": "string", "description": "Optional arguments" }
            },
            "required": ["skill"]
        })
    }

    async fn call(
        &self,
        _input: serde_json::Value,
        _context: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        Err(ToolError::ExecutionFailed(
            "Skill execution is not yet implemented".to_string(),
        ))
    }
}
