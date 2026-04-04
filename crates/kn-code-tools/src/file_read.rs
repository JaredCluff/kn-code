use crate::traits::*;
use async_trait::async_trait;
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug)]
pub struct FileReadTool;

impl Default for FileReadTool {
    fn default() -> Self {
        Self
    }
}

#[derive(Debug, Deserialize)]
struct FileReadInput {
    file_path: String,
    #[serde(default)]
    offset: Option<usize>,
    #[serde(default)]
    limit: Option<usize>,
}

#[async_trait]
impl Tool for FileReadTool {
    fn name(&self) -> &str {
        "FileRead"
    }
    fn description(&self) -> &str {
        "Read the contents of a file"
    }
    fn prompt(&self) -> &str {
        "Use this to read file contents."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "file_path": { "type": "string", "description": "Absolute path to the file" },
                "offset": { "type": "integer", "description": "Line number to start from (1-indexed)" },
                "limit": { "type": "integer", "description": "Number of lines to read" }
            },
            "required": ["file_path"]
        })
    }

    fn is_read_only(&self) -> bool {
        true
    }

    async fn call(
        &self,
        input: serde_json::Value,
        context: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let parsed: FileReadInput =
            serde_json::from_value(input.clone()).map_err(|e| ToolError::ValidationFailed {
                message: e.to_string(),
            })?;

        let requested = PathBuf::from(&parsed.file_path);
        let path = if requested.is_absolute() {
            requested
        } else {
            context.cwd.join(&requested)
        };

        let canonical = path
            .canonicalize()
            .map_err(|_| ToolError::PermissionDenied {
                message: "File not found or not accessible".to_string(),
            })?;

        let cwd = context
            .cwd
            .canonicalize()
            .map_err(|_| ToolError::PermissionDenied {
                message: "Cannot resolve working directory".to_string(),
            })?;

        if !canonical.starts_with(&cwd) {
            return Ok(ToolResult {
                content: ToolContent::Text(
                    "Error: file is outside the working directory. Access to files outside the project directory is not allowed.".to_string(),
                ),
                new_messages: Vec::new(),
                persisted: false,
                persisted_path: None,
                structured_content: None,
            });
        }

        let path_str = canonical.to_string_lossy();
        if path_str.starts_with("/dev/")
            && !path_str.starts_with("/dev/null")
            && !path_str.starts_with("/dev/zero")
        {
            return Ok(ToolResult {
                content: ToolContent::Text(format!(
                    "Reading from {} is not allowed.",
                    canonical.display()
                )),
                new_messages: Vec::new(),
                persisted: false,
                persisted_path: None,
                structured_content: None,
            });
        }

        let content = tokio::fs::read_to_string(&canonical)
            .await
            .map_err(ToolError::Io)?;

        let lines: Vec<&str> = content.lines().collect();
        let total_lines = lines.len();

        let offset = parsed.offset.unwrap_or(1).saturating_sub(1);
        let limit = parsed.limit.unwrap_or(total_lines);
        let end = (offset + limit).min(total_lines);

        let selected_lines = if offset < total_lines {
            &lines[offset..end]
        } else {
            &[]
        };

        let output = if selected_lines.is_empty() {
            format!(
                "File is empty or offset ({}) exceeds total lines ({}).",
                offset + 1,
                total_lines
            )
        } else {
            let displayed = selected_lines.join("\n");
            let header = format!(
                "Read lines {}-{} of {} from {}:\n",
                offset + 1,
                end,
                total_lines,
                canonical.display()
            );
            format!("{}{}", header, displayed)
        };

        Ok(ToolResult {
            content: ToolContent::Text(output),
            new_messages: Vec::new(),
            persisted: false,
            persisted_path: None,
            structured_content: Some(serde_json::json!({
                "file_path": parsed.file_path,
                "total_lines": total_lines,
                "lines_read": selected_lines.len(),
                "offset": offset + 1,
            })),
        })
    }

    fn get_path(&self, input: &serde_json::Value) -> Option<PathBuf> {
        input
            .get("file_path")
            .and_then(|v| v.as_str())
            .map(PathBuf::from)
    }
}
