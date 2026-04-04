use crate::traits::*;
use async_trait::async_trait;
use serde::Deserialize;

#[derive(Debug)]
pub struct LspTool;

impl Default for LspTool {
    fn default() -> Self {
        Self
    }
}

#[derive(Debug, Deserialize)]
struct LspInput {
    #[serde(default)]
    action: Option<String>,
    #[serde(default)]
    file_path: Option<String>,
}

#[derive(Debug, serde::Serialize)]
#[allow(dead_code)]
struct LspDiagnostic {
    line: u32,
    column: u32,
    severity: String,
    message: String,
    source: String,
}

#[async_trait]
impl Tool for LspTool {
    fn name(&self) -> &str {
        "LSP"
    }
    fn description(&self) -> &str {
        "Query LSP diagnostics for a file"
    }
    fn prompt(&self) -> &str {
        "Use this to check for LSP diagnostics (errors, warnings) in files."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["diagnostics", "hover", "definition"], "description": "LSP action to perform" },
                "file_path": { "type": "string", "description": "File to check" }
            },
            "required": ["action"]
        })
    }

    fn is_read_only(&self) -> bool {
        true
    }

    async fn call(
        &self,
        input: serde_json::Value,
        _context: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let parsed: LspInput =
            serde_json::from_value(input.clone()).map_err(|e| ToolError::ValidationFailed {
                message: e.to_string(),
            })?;

        let action = parsed.action.as_deref().unwrap_or("diagnostics");

        // TODO: Connect to actual LSP server
        // For now, return a placeholder
        Ok(ToolResult {
            content: ToolContent::Text(format!(
                "LSP {} requested for {:?} — LSP server integration not yet configured.",
                action, parsed.file_path
            )),
            new_messages: Vec::new(),
            persisted: false,
            persisted_path: None,
            structured_content: Some(serde_json::json!({
                "action": action,
                "file_path": parsed.file_path,
                "status": "not_configured",
                "diagnostics": [],
            })),
        })
    }
}
