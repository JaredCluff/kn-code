use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use kn_code_auth::FileTokenStore;
use kn_code_permissions::rules::PermissionMode;
use kn_code_providers::resolve_provider;
use kn_code_session::SessionStore;
use kn_code_session::messages::{ContentBlock, Message, UserMessage};
use kn_code_session::runner::AgentRunner;
use kn_code_tools::traits::Tool;
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
    pub token_store: Arc<FileTokenStore>,
    pub tools: Vec<Arc<dyn Tool>>,
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

    if !cwd.is_absolute() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "cwd must be an absolute path",
            })),
        );
    }

    let cwd = cwd.canonicalize().unwrap_or(cwd);
    if !cwd.exists() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": format!("cwd does not exist: {}", cwd.display()),
            })),
        );
    }
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

    let session_store = state.0.session_store.clone();
    let token_store = state.0.token_store.clone();
    let tools = state.0.tools.clone();
    let permission_mode = match req.permission_mode.as_deref() {
        Some("auto") => PermissionMode::Auto,
        Some("ask") => PermissionMode::Ask,
        Some("bypass") => PermissionMode::BypassPermissions,
        _ => PermissionMode::BypassPermissions,
    };
    let max_turns = req.max_turns.unwrap_or(50);
    let session_id_clone = session_id.clone();
    let cwd_clone = cwd.clone();
    let model_clone = model.clone();
    let session_store_for_runner = session_store.clone();

    tokio::spawn(async move {
        let Some((provider, model_info)) = resolve_provider(&model_clone) else {
            tracing::error!(
                model = model_clone,
                "Unknown provider prefix — supported: anthropic, openai, github_copilot"
            );
            let _ = session_store
                .update_session_state(&session_id_clone, "error")
                .await;
            return;
        };
        let runner = AgentRunner {
            session_store: session_store_for_runner,
            token_store,
            provider,
            tools,
            permission_mode,
            max_turns,
            cwd: cwd_clone,
            model_info,
            cancellation_token: None,
        };

        match runner.run(&session_id_clone).await {
            Ok(result) => {
                tracing::info!(
                    "Agent run completed: session={}, turns={}, stop={}, tokens_in={}, tokens_out={}",
                    result.session_id,
                    result.turns_completed,
                    result.stop_reason,
                    result.input_tokens,
                    result.output_tokens,
                );
            }
            Err(e) => {
                tracing::error!("Agent run failed for session {}: {}", session_id_clone, e);
                let _ = session_store
                    .update_session_state(&session_id_clone, "error")
                    .await;
            }
        }
    });

    (
        StatusCode::ACCEPTED,
        Json(serde_json::json!({
            "session_id": session_id,
            "status": "running",
            "model": model,
            "cwd": cwd.to_string_lossy(),
            "message": "Agent started",
        })),
    )
}
