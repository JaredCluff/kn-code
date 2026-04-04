use crate::traits::*;
use async_trait::async_trait;
use ignore::WalkBuilder;
use regex::RegexBuilder;
use serde::Deserialize;
use std::path::PathBuf;

const MAX_FILE_SIZE: usize = 10 * 1024 * 1024;

#[derive(Debug)]
pub struct GrepTool;

impl Default for GrepTool {
    fn default() -> Self {
        Self
    }
}

#[derive(Debug, Deserialize)]
struct GrepInput {
    pattern: String,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    output_mode: Option<String>,
}

#[async_trait]
impl Tool for GrepTool {
    fn name(&self) -> &str {
        "Grep"
    }
    fn description(&self) -> &str {
        "Search file contents with regex"
    }
    fn prompt(&self) -> &str {
        "Use this to search file contents with regex. Respects .gitignore."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string", "description": "Regex pattern" },
                "path": { "type": "string", "description": "File or directory to search in (optional, defaults to cwd)" },
                "output_mode": { "type": "string", "enum": ["content", "files_with_matches", "count"] }
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
        let parsed: GrepInput =
            serde_json::from_value(input.clone()).map_err(|e| ToolError::ValidationFailed {
                message: e.to_string(),
            })?;

        let re = RegexBuilder::new(&parsed.pattern)
            .size_limit(1_000_000)
            .build()
            .map_err(|e| ToolError::ValidationFailed {
                message: format!("Invalid regex: {}", e),
            })?;

        let cwd = context
            .cwd
            .canonicalize()
            .map_err(|e| ToolError::PermissionDenied {
                message: format!("Cannot resolve working directory: {}", e),
            })?;

        let search_path = if let Some(p) = &parsed.path {
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
                    content: ToolContent::Text(format!("Path not found: {}", path.display())),
                    new_messages: Vec::new(),
                    persisted: false,
                    persisted_path: None,
                    structured_content: None,
                });
            }
        } else {
            cwd.clone()
        };

        if !search_path.starts_with(&cwd) {
            return Ok(ToolResult {
                content: ToolContent::Text(format!(
                    "Error: search path '{}' is outside the working directory '{}'.",
                    search_path.display(),
                    cwd.display()
                )),
                new_messages: Vec::new(),
                persisted: false,
                persisted_path: None,
                structured_content: None,
            });
        }

        let output_mode = parsed.output_mode.as_deref().unwrap_or("content");

        let walker = WalkBuilder::new(&search_path)
            .hidden(true)
            .git_ignore(true)
            .max_depth(Some(25))
            .build();

        let mut results: Vec<String> = Vec::new();
        let mut file_count = 0;
        let mut match_count = 0;
        const MAX_RESULTS: usize = 1000;

        for entry in walker.filter_map(|e| e.ok()) {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }

            let metadata = path.metadata();
            if let Ok(meta) = metadata
                && meta.len() > MAX_FILE_SIZE as u64 {
                    continue;
                }

            if let Ok(content) = tokio::fs::read_to_string(path).await {
                let mut file_matches: Vec<String> = Vec::new();
                for (line_num, line) in content.lines().enumerate() {
                    if re.is_match(line) {
                        match_count += 1;
                        if output_mode == "content" {
                            file_matches.push(format!("{}:{}", line_num + 1, line));
                        }
                    }
                }

                if !file_matches.is_empty() {
                    file_count += 1;
                    match output_mode {
                        "content" => {
                            let rel = path.strip_prefix(&cwd).unwrap_or(path).display();
                            for m in file_matches {
                                results.push(format!("{}:{}", rel, m));
                            }
                        }
                        "files_with_matches" => {
                            results.push(path.display().to_string());
                        }
                        "count" => {
                            results.push(format!("{}:{}", path.display(), file_matches.len()));
                        }
                        _ => {}
                    }
                }
            }

            if results.len() >= MAX_RESULTS {
                results.push(format!("... (truncated to {} results)", MAX_RESULTS));
                break;
            }
        }

        let output = results.join("\n");

        Ok(ToolResult {
            content: ToolContent::Text(if output.is_empty() {
                "No matches found.".to_string()
            } else {
                format!(
                    "Found {} match(es) in {} file(s):\n{}",
                    match_count, file_count, output
                )
            }),
            new_messages: Vec::new(),
            persisted: false,
            persisted_path: None,
            structured_content: Some(serde_json::json!({
                "match_count": match_count,
                "file_count": file_count,
                "output_mode": output_mode,
            })),
        })
    }
}
