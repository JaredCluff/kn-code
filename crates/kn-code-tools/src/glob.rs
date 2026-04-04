use crate::traits::*;
use async_trait::async_trait;
use globset::GlobBuilder;
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug)]
pub struct GlobTool;

impl Default for GlobTool {
    fn default() -> Self {
        Self
    }
}

#[derive(Debug, Deserialize)]
struct GlobInput {
    pattern: String,
    #[serde(default)]
    path: Option<String>,
}

#[async_trait]
impl Tool for GlobTool {
    fn name(&self) -> &str {
        "Glob"
    }
    fn description(&self) -> &str {
        "Find files matching a glob pattern"
    }
    fn prompt(&self) -> &str {
        "Use this to find files by pattern."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string", "description": "Glob pattern" },
                "path": { "type": "string", "description": "Directory to search in (optional, defaults to cwd)" }
            },
            "required": ["pattern"]
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
        let parsed: GlobInput =
            serde_json::from_value(input.clone()).map_err(|e| ToolError::ValidationFailed {
                message: e.to_string(),
            })?;

        let cwd = context
            .cwd
            .canonicalize()
            .map_err(|e| ToolError::PermissionDenied {
                message: format!("Cannot resolve working directory: {}", e),
            })?;

        let search_dir = if let Some(p) = &parsed.path {
            let requested = PathBuf::from(p);
            let path = if requested.is_absolute() {
                requested
            } else {
                cwd.join(&requested)
            };
            if path.exists() {
                path.canonicalize()
                    .map_err(|e| ToolError::PermissionDenied {
                        message: format!("Cannot resolve search path: {}", e),
                    })?
            } else {
                return Ok(ToolResult {
                    content: ToolContent::Text(format!("Directory not found: {}", path.display())),
                    new_messages: Vec::new(),
                    persisted: false,
                    persisted_path: None,
                    structured_content: None,
                });
            }
        } else {
            cwd.clone()
        };

        if !search_dir.starts_with(&cwd) {
            return Ok(ToolResult {
                content: ToolContent::Text(format!(
                    "Error: search path '{}' is outside the working directory '{}'.",
                    search_dir.display(),
                    cwd.display()
                )),
                new_messages: Vec::new(),
                persisted: false,
                persisted_path: None,
                structured_content: None,
            });
        }

        let glob = GlobBuilder::new(&parsed.pattern)
            .case_insensitive(false)
            .build()
            .map_err(|e| ToolError::ValidationFailed {
                message: e.to_string(),
            })?;

        let matcher = glob.compile_matcher();
        let mut matches = Vec::new();
        const MAX_RESULTS: usize = 1000;
        const MAX_DEPTH: usize = 15;

        let skip_dirs = [
            ".git",
            "node_modules",
            "target",
            ".venv",
            "vendor",
            "__pycache__",
            ".next",
            "dist",
            "build",
        ];

        for entry in walkdir::WalkDir::new(&search_dir)
            .follow_links(false)
            .max_depth(MAX_DEPTH)
            .into_iter()
            .filter_entry(|e| {
                let name = e.file_name().to_string_lossy();
                !skip_dirs.contains(&name.as_ref())
            })
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if let Ok(relative) = path.strip_prefix(&search_dir)
                && matcher.is_match(relative) {
                    matches.push(path.to_string_lossy().to_string());
                    if matches.len() >= MAX_RESULTS {
                        break;
                    }
                }
        }

        matches.sort();

        if matches.is_empty() {
            Ok(ToolResult {
                content: ToolContent::Text(format!(
                    "No files matching pattern: {}",
                    parsed.pattern
                )),
                new_messages: Vec::new(),
                persisted: false,
                persisted_path: None,
                structured_content: Some(serde_json::json!({"matches": []})),
            })
        } else {
            Ok(ToolResult {
                content: ToolContent::Text(format!(
                    "Found {} file(s) matching '{}':\n{}",
                    matches.len(),
                    parsed.pattern,
                    matches.join("\n")
                )),
                new_messages: Vec::new(),
                persisted: false,
                persisted_path: None,
                structured_content: Some(serde_json::json!({"matches": matches})),
            })
        }
    }
}
