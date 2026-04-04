use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use kn_code_session::SessionStore;
use kn_code_session::messages::{ContentBlock, Message, UserMessage};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

const MAX_PROMPT_LENGTH: usize = 50_000;
const MAX_TURNS: u64 = 100;
const MAX_ENV_VARS: usize = 20;
const ALLOWED_ENV_PREFIXES: &[&str] = &[
    "PATH", "HOME", "USER", "LANG", "TERM", "NODE_", "PYTHON", "GO", "RUST_", "CI_", "NPM_",
];

#[derive(Deserialize)]
pub struct RunRequest {
    pub prompt: String,
    pub cwd: Option<String>,
    pub model: Option<String>,
    pub variant: Option<String>,
    pub session_id: Option<String>,
    pub permission_mode: Option<String>,
    pub max_turns: Option<u64>,
    pub timeout_seconds: Option<u64>,
    pub stream: Option<bool>,
    pub env: Option<HashMap<String, String>>,
}

impl RunRequest {
    pub fn validate(&self) -> Result<(), String> {
        if self.prompt.is_empty() {
            return Err("Prompt cannot be empty".to_string());
        }
        if self.prompt.len() > MAX_PROMPT_LENGTH {
            return Err(format!(
                "Prompt exceeds maximum length of {} characters",
                MAX_PROMPT_LENGTH
            ));
        }
        if let Some(max_turns) = self.max_turns
            && max_turns > MAX_TURNS
        {
            return Err(format!("max_turns exceeds maximum of {}", MAX_TURNS));
        }
        if let Some(env) = &self.env {
            if env.len() > MAX_ENV_VARS {
                return Err(format!(
                    "Too many environment variables (max {})",
                    MAX_ENV_VARS
                ));
            }
            for key in env.keys() {
                if !ALLOWED_ENV_PREFIXES
                    .iter()
                    .any(|prefix| key.starts_with(*prefix))
                {
                    return Err(format!("Environment variable '{}' is not allowed", key));
                }
            }
        }
        Ok(())
    }
}

#[derive(Serialize)]
pub struct RunResponse {
    pub session_id: String,
    pub exit_code: i32,
    pub summary: String,
}

#[derive(Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

pub struct RunState {
    pub session_store: Arc<SessionStore>,
}

pub async fn run_agent(
    state: State<Arc<RunState>>,
    Json(req): Json<RunRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    if let Err(e) = req.validate() {
        tracing::warn!(error = %e, "RunRequest validation failed");
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": format!("Validation error: {}", e),
            })),
        );
    }

    let cwd = req
        .cwd
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let model = req
        .model
        .as_deref()
        .unwrap_or("anthropic/claude-sonnet-4-5")
        .to_string();

    let session_id = match &req.session_id {
        Some(id) => id.clone(),
        None => {
            match state
                .0
                .session_store
                .create_session(cwd.clone(), model.clone())
                .await
            {
                Ok(record) => record.id,
                Err(e) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({
                            "error": format!("Failed to create session: {}", e),
                        })),
                    );
                }
            }
        }
    };

    let message = Message::User(UserMessage {
        id: uuid::Uuid::new_v4().to_string(),
        content: vec![ContentBlock::Text(req.prompt.clone())],
        timestamp: chrono::Utc::now(),
    });

    match state
        .0
        .session_store
        .append_message(&session_id, &message)
        .await
    {
        Ok(()) => {}
        Err(e) => {
            tracing::error!("Failed to append message to session {}: {}", session_id, e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": format!("Failed to store prompt message: {}", e),
                })),
            );
        }
    }

    (
        StatusCode::ACCEPTED,
        Json(serde_json::json!({
            "session_id": session_id,
            "status": "accepted",
            "model": model,
            "cwd": cwd.to_string_lossy(),
            "message": "Session created and prompt queued for processing",
        })),
    )
}
