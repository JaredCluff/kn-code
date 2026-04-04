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
pub struct FileWriteTool;

impl Default for FileWriteTool {
    fn default() -> Self {
        Self
    }
}

#[derive(Debug, Deserialize)]
struct FileWriteInput {
    file_path: String,
    content: String,
}

#[derive(Debug, serde::Serialize)]
struct FileWriteOutput {
    r#type: String,
    file_path: String,
    content_length: usize,
}

fn validate_path_within_cwd(
    path: &std::path::Path,
    cwd: &std::path::Path,
) -> Result<PathBuf, ToolError> {
    if !path.starts_with(cwd) {
        return Err(ToolError::PermissionDenied {
            message: format!(
                "Error: path '{}' is outside the working directory '{}'. Access to files outside the project directory is not allowed.",
                path.display(),
                cwd.display()
            ),
        });
    }
    Ok(path.to_path_buf())
}

#[async_trait]
impl Tool for FileWriteTool {
    fn name(&self) -> &str {
        "FileWrite"
    }
    fn description(&self) -> &str {
        "Write content to a file"
    }
    fn prompt(&self) -> &str {
        "Use this to create or overwrite files."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "file_path": { "type": "string", "description": "Path relative to working directory or absolute" },
                "content": { "type": "string", "description": "Content to write" }
            },
            "required": ["file_path", "content"]
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
        let parsed: FileWriteInput =
            serde_json::from_value(input.clone()).map_err(|e| ToolError::ValidationFailed {
                message: e.to_string(),
            })?;

        if parsed.content.len() > MAX_FILE_SIZE {
            return Ok(ToolResult {
                content: ToolContent::Text(format!(
                    "Content too large ({} bytes, max {} bytes)",
                    parsed.content.len(),
                    MAX_FILE_SIZE
                )),
                new_messages: Vec::new(),
                persisted: false,
                persisted_path: None,
                structured_content: None,
            });
        }

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
                    return Err(ToolError::PermissionDenied {
                        message: format!(
                            "Error: path '{}' is outside the working directory '{}'. Access to files outside the project directory is not allowed.",
                            path.display(),
                            cwd.display()
                        ),
                    });
                }
                normalized
            }
        };

        validate_path_within_cwd(&resolved, &cwd)?;

        let existed = resolved.exists();

        if let Some(parent) = resolved.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(ToolError::Io)?;
        }

        let temp_path = resolved.with_extension(format!(
            "kn-tmp.{}.{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));

        tokio::fs::write(&temp_path, &parsed.content)
            .await
            .map_err(ToolError::Io)?;

        if let Err(e) = tokio::fs::rename(&temp_path, &resolved).await {
            let _ = tokio::fs::remove_file(&temp_path).await;
            return Err(ToolError::Io(e));
        }

        let r#type = if existed { "update" } else { "create" };

        Ok(ToolResult {
            content: ToolContent::Text(
                serde_json::to_string(&FileWriteOutput {
                    r#type: r#type.to_string(),
                    file_path: parsed.file_path.clone(),
                    content_length: parsed.content.len(),
                })
                .unwrap_or_default(),
            ),
            new_messages: Vec::new(),
            persisted: false,
            persisted_path: None,
            structured_content: Some(serde_json::json!({
                "type": r#type,
                "file_path": parsed.file_path,
                "content_length": parsed.content.len(),
            })),
        })
    }
}
