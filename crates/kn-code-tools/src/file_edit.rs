use crate::traits::*;
use async_trait::async_trait;
use serde::Deserialize;
use std::path::{Path, PathBuf};

const MAX_FILE_SIZE: usize = 10 * 1024 * 1024;

fn normalize_path(cwd: &Path, path: &Path) -> Result<PathBuf, ToolError> {
    let mut components = Vec::new();
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    };

    for component in joined.components() {
        match component {
            std::path::Component::ParentDir => {
                if components.pop().is_none() {
                    return Err(ToolError::PermissionDenied {
                        message: "Path escapes working directory via '..'".to_string(),
                    });
                }
            }
            std::path::Component::Normal(c) => {
                components.push(c);
            }
            std::path::Component::CurDir | std::path::Component::RootDir => {}
            std::path::Component::Prefix(_) => {
                return Err(ToolError::PermissionDenied {
                    message: "Path contains a prefix (e.g., drive letter)".to_string(),
                });
            }
        }
    }

    let mut result = if path.is_absolute() {
        PathBuf::new()
    } else {
        cwd.to_path_buf()
    };
    for c in components {
        result.push(c);
    }
    Ok(result)
}

#[derive(Debug)]
pub struct FileEditTool;

impl Default for FileEditTool {
    fn default() -> Self {
        Self
    }
}

#[derive(Debug, Deserialize)]
struct FileEditInput {
    file_path: String,
    old_string: String,
    new_string: String,
    #[serde(default)]
    replace_all: Option<bool>,
}

#[async_trait]
impl Tool for FileEditTool {
    fn name(&self) -> &str {
        "FileEdit"
    }
    fn description(&self) -> &str {
        "Edit a file by finding and replacing text"
    }
    fn prompt(&self) -> &str {
        "Use this to edit files by replacing text."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "file_path": { "type": "string" },
                "old_string": { "type": "string" },
                "new_string": { "type": "string" },
                "replace_all": { "type": "boolean" }
            },
            "required": ["file_path", "old_string", "new_string"]
        })
    }

    fn is_destructive(&self) -> bool {
        true
    }

    async fn call(
        &self,
        input: serde_json::Value,
        context: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let parsed: FileEditInput =
            serde_json::from_value(input.clone()).map_err(|e| ToolError::ValidationFailed {
                message: e.to_string(),
            })?;

        let requested = PathBuf::from(&parsed.file_path);
        let path = if requested.is_absolute() {
            requested
        } else {
            context.cwd.join(&requested)
        };

        let cwd = context
            .cwd
            .canonicalize()
            .map_err(|e| ToolError::PermissionDenied {
                message: format!("Cannot resolve working directory: {}", e),
            })?;

        let resolved = if path.exists() {
            path.canonicalize()
                .map_err(|e| ToolError::PermissionDenied {
                    message: format!("Cannot resolve file path: {}", e),
                })?
        } else {
            let parent = path.parent().unwrap_or(&path);
            if parent.exists() {
                let canonical_parent =
                    parent
                        .canonicalize()
                        .map_err(|e| ToolError::PermissionDenied {
                            message: format!("Cannot resolve parent directory: {}", e),
                        })?;
                let file_name = path
                    .file_name()
                    .ok_or_else(|| ToolError::ValidationFailed {
                        message: "Path has no file name component".to_string(),
                    })?;
                canonical_parent.join(file_name)
            } else {
                let normalized = normalize_path(&cwd, &path)?;
                if !normalized.starts_with(&cwd) {
                    return Ok(ToolResult {
                        content: ToolContent::Text(format!(
                            "Error: path '{}' is outside the working directory '{}'. Access to files outside the project directory is not allowed.",
                            path.display(),
                            cwd.display()
                        )),
                        new_messages: Vec::new(),
                        persisted: false,
                        persisted_path: None,
                        structured_content: None,
                    });
                }
                normalized
            }
        };

        if !resolved.starts_with(&cwd) {
            return Ok(ToolResult {
                content: ToolContent::Text(format!(
                    "Error: path '{}' is outside the working directory '{}'. Access to files outside the project directory is not allowed.",
                    resolved.display(),
                    cwd.display()
                )),
                new_messages: Vec::new(),
                persisted: false,
                persisted_path: None,
                structured_content: None,
            });
        }

        let content = tokio::fs::read_to_string(&resolved)
            .await
            .map_err(ToolError::Io)?;

        if content.len() > MAX_FILE_SIZE {
            return Ok(ToolResult {
                content: ToolContent::Text(format!(
                    "File too large to edit ({} bytes, max {} bytes)",
                    content.len(),
                    MAX_FILE_SIZE
                )),
                new_messages: Vec::new(),
                persisted: false,
                persisted_path: None,
                structured_content: None,
            });
        }

        let old_line_count = content.lines().count() as i64;

        let replace_all = parsed.replace_all.unwrap_or(false);
        let new_content = if replace_all {
            content.replace(&parsed.old_string, &parsed.new_string)
        } else {
            match content.find(&parsed.old_string) {
                Some(pos) => {
                    let mut result = content.clone();
                    result.replace_range(pos..pos + parsed.old_string.len(), &parsed.new_string);
                    result
                }
                None => {
                    return Ok(ToolResult {
                        content: ToolContent::Text("String not found in file.".to_string()),
                        new_messages: Vec::new(),
                        persisted: false,
                        persisted_path: None,
                        structured_content: None,
                    });
                }
            }
        };

        if new_content.len() > MAX_FILE_SIZE {
            return Ok(ToolResult {
                content: ToolContent::Text(format!(
                    "Result too large after replacement ({} bytes, max {} bytes)",
                    new_content.len(),
                    MAX_FILE_SIZE
                )),
                new_messages: Vec::new(),
                persisted: false,
                persisted_path: None,
                structured_content: None,
            });
        }

        let replace_count = if replace_all {
            content.matches(&parsed.old_string).count()
        } else if content.contains(&parsed.old_string) {
            1
        } else {
            0
        };

        if replace_count == 0 {
            return Ok(ToolResult {
                content: ToolContent::Text(
                    "No changes made — old_string not found in file.".to_string(),
                ),
                new_messages: Vec::new(),
                persisted: false,
                persisted_path: None,
                structured_content: None,
            });
        }

        let temp_path = resolved.with_extension(format!(
            "kn-tmp.{}.{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        tokio::fs::write(&temp_path, &new_content)
            .await
            .map_err(ToolError::Io)?;
        if let Err(e) = tokio::fs::rename(&temp_path, &resolved).await {
            let _ = tokio::fs::remove_file(&temp_path).await;
            return Err(ToolError::Io(e));
        }

        let diff_lines = new_content.lines().count() as i64 - old_line_count;

        Ok(ToolResult {
            content: ToolContent::Text(format!(
                "Edited {}: replaced {} occurrence(s) of string.",
                resolved.display(),
                replace_count
            )),
            new_messages: Vec::new(),
            persisted: false,
            persisted_path: None,
            structured_content: Some(serde_json::json!({
                "file_path": parsed.file_path,
                "replacements": replace_count,
                "line_delta": diff_lines,
            })),
        })
    }
}
