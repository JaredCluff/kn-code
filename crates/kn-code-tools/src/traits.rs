use async_trait::async_trait;
use kn_code_permissions::PermissionDecision;
use serde::{Deserialize, Serialize};
use std::fmt::Debug;
use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct ToolContext {
    pub cwd: PathBuf,
    pub is_headless: bool,
    pub session_id: Option<String>,
    pub tool_use_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ToolContent {
    Text(String),
    Image { base64: String, media_type: String },
    Multi(Vec<ContentBlock>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ContentBlock {
    Text(String),
    Image { base64: String, media_type: String },
    Error { message: String, code: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub content: ToolContent,
    pub new_messages: Vec<String>,
    pub persisted: bool,
    pub persisted_path: Option<PathBuf>,
    pub structured_content: Option<serde_json::Value>,
}

#[derive(Debug, Error)]
pub enum ToolError {
    #[error("Validation failed: {message}")]
    ValidationFailed { message: String },
    #[error("Permission denied: {message}")]
    PermissionDenied { message: String },
    #[error("Execution failed: {0}")]
    ExecutionFailed(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

#[async_trait]
pub trait Tool: Send + Sync + Debug {
    fn name(&self) -> &str;
    fn aliases(&self) -> &[&str] {
        &[]
    }
    fn description(&self) -> &str;
    fn prompt(&self) -> &str;
    fn input_schema(&self) -> serde_json::Value;
    fn output_schema(&self) -> Option<serde_json::Value> {
        None
    }
    fn is_enabled(&self) -> bool {
        true
    }
    fn is_concurrency_safe(&self) -> bool {
        false
    }
    fn is_read_only(&self) -> bool {
        false
    }
    fn is_destructive(&self) -> bool {
        false
    }
    fn max_result_size_chars(&self) -> usize {
        100_000
    }
    fn strict_schema(&self) -> bool {
        true
    }

    async fn check_permission(
        &self,
        _input: &serde_json::Value,
        _context: &ToolContext,
    ) -> anyhow::Result<PermissionDecision> {
        Ok(PermissionDecision::Allow {
            updated_input: None,
            reason: kn_code_permissions::rules::PermissionReason::Other,
        })
    }

    async fn call(
        &self,
        input: serde_json::Value,
        context: ToolContext,
    ) -> Result<ToolResult, ToolError>;

    fn get_path(&self, _input: &serde_json::Value) -> Option<PathBuf> {
        None
    }
}
