use serde::{Deserialize, Serialize};

/// All event types that kn-code emits in headless mode.
/// These must be compatible with Paperclip's parseOpenCodeJsonl() parser.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SdkEvent {
    /// Session initialization
    System {
        subtype: String,
        session_id: String,
        model: String,
    },
    /// Assistant text output
    Text { content: String },
    /// Tool usage by the LLM
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    /// Tool result
    ToolResult {
        id: String,
        output: serde_json::Value,
    },
    /// Step completion with usage tracking (CRITICAL for Paperclip)
    StepFinish { usage: TokenUsage, cost_usd: f64 },
    /// Permission request (when not in bypass mode)
    PermissionRequest {
        tool_name: String,
        input: serde_json::Value,
        message: String,
        request_id: String,
    },
    /// Session state change
    SessionState {
        state: String,
        turns_completed: u64,
        cost_usd: f64,
    },
    /// Final result (REQUIRED — tells orchestrator the run is done)
    Result {
        subtype: String,
        session_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        usage: Option<TokenUsage>,
        #[serde(skip_serializing_if = "Option::is_none")]
        cost_usd: Option<f64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        summary: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    /// Error event
    Error {
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        code: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    #[serde(default)]
    pub cache_read_tokens: u64,
    #[serde(default)]
    pub reasoning_tokens: u64,
}

impl SdkEvent {
    pub fn session_init(session_id: &str, model: &str) -> Self {
        Self::System {
            subtype: "init".to_string(),
            session_id: session_id.to_string(),
            model: model.to_string(),
        }
    }

    pub fn text(content: &str) -> Self {
        Self::Text {
            content: content.to_string(),
        }
    }

    pub fn tool_use(id: &str, name: &str, input: serde_json::Value) -> Self {
        Self::ToolUse {
            id: id.to_string(),
            name: name.to_string(),
            input,
            error: None,
        }
    }

    pub fn tool_use_error(id: &str, name: &str, error: &str) -> Self {
        Self::ToolUse {
            id: id.to_string(),
            name: name.to_string(),
            input: serde_json::json!({}),
            error: Some(error.to_string()),
        }
    }

    pub fn tool_result(id: &str, output: serde_json::Value) -> Self {
        Self::ToolResult {
            id: id.to_string(),
            output,
        }
    }

    pub fn step_finish(usage: TokenUsage, cost_usd: f64) -> Self {
        Self::StepFinish { usage, cost_usd }
    }

    pub fn permission_request(
        tool_name: &str,
        input: serde_json::Value,
        message: &str,
        request_id: &str,
    ) -> Self {
        Self::PermissionRequest {
            tool_name: tool_name.to_string(),
            input,
            message: message.to_string(),
            request_id: request_id.to_string(),
        }
    }

    pub fn session_state(state: &str, turns_completed: u64, cost_usd: f64) -> Self {
        Self::SessionState {
            state: state.to_string(),
            turns_completed,
            cost_usd,
        }
    }

    pub fn result_success(
        session_id: &str,
        usage: TokenUsage,
        cost_usd: f64,
        summary: String,
    ) -> Self {
        Self::Result {
            subtype: "success".to_string(),
            session_id: session_id.to_string(),
            usage: Some(usage),
            cost_usd: Some(cost_usd),
            summary: Some(summary),
            error: None,
        }
    }

    pub fn result_error(session_id: &str, error: String) -> Self {
        Self::Result {
            subtype: "error".to_string(),
            session_id: session_id.to_string(),
            usage: None,
            cost_usd: None,
            summary: None,
            error: Some(error),
        }
    }

    pub fn error(message: &str, code: Option<&str>) -> Self {
        Self::Error {
            message: message.to_string(),
            code: code.map(String::from),
        }
    }

    /// Unknown session error — Paperclip uses this regex to detect:
    /// /unknown\s+session|session\b.*\bnot\s+found|resource\s+not\s+found:.*[\\/]session[\\/].*\.json|notfounderror|no session/i
    pub fn unknown_session(session_id: &str) -> Self {
        Self::Error {
            message: format!("unknown session {}", session_id),
            code: Some("SESSION_NOT_FOUND".to_string()),
        }
    }
}
