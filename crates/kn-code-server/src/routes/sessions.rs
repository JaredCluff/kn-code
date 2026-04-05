use axum::Json;
use axum::extract::Path;
use kn_code_session::SessionStore;
use serde::Serialize;
use std::sync::Arc;

fn is_valid_session_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
        && !id.contains("..")
        && !id.starts_with('.')
        && !id.starts_with('/')
}

#[derive(Serialize)]
pub struct SessionInfo {
    pub session_id: String,
    pub status: String,
    pub model: String,
    pub turns_completed: u64,
}

pub struct SessionState {
    pub session_store: Arc<SessionStore>,
}

pub async fn list_sessions(
    state: axum::extract::State<Arc<SessionState>>,
) -> Json<Vec<SessionInfo>> {
    let store = &state.0.session_store;
    if !store.base_dir.exists() {
        return Json(Vec::new());
    }

    let mut sessions = Vec::new();
    match tokio::fs::read_dir(&store.base_dir).await {
        Ok(mut entries) => {
            while let Ok(Some(entry)) = entries.next_entry().await {
                if entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false) {
                    let session_id = entry.file_name().to_string_lossy().to_string();
                    if let Ok(Some(record)) = store.load_session(&session_id).await {
                        sessions.push(SessionInfo {
                            session_id: record.id,
                            status: record.state,
                            model: record.model,
                            turns_completed: record.turns_completed,
                        });
                    }
                }
            }
        }
        Err(e) => {
            tracing::error!("Failed to read session directory: {}", e);
        }
    }

    Json(sessions)
}

pub async fn get_session(
    state: axum::extract::State<Arc<SessionState>>,
    Path(session_id): Path<String>,
) -> (axum::http::StatusCode, Json<serde_json::Value>) {
    let store = &state.0.session_store;
    match store.load_session(&session_id).await {
        Ok(Some(record)) => (
            axum::http::StatusCode::OK,
            Json(serde_json::json!({
                "session_id": record.id,
                "status": record.state,
                "model": record.model,
                "cwd": record.cwd.to_string_lossy(),
                "turns_completed": record.turns_completed,
                "cost_usd": record.cost_usd,
                "created_at": record.created_at,
                "updated_at": record.updated_at,
            })),
        ),
        Ok(None) => (
            axum::http::StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": format!("Session not found: {}", session_id),
            })),
        ),
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": format!("Failed to load session: {}", e),
            })),
        ),
    }
}

pub async fn cancel_session(
    state: axum::extract::State<Arc<SessionState>>,
    Path(session_id): Path<String>,
) -> (axum::http::StatusCode, Json<serde_json::Value>) {
    if !is_valid_session_id(&session_id) {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "Invalid session_id format",
            })),
        );
    }

    let store = &state.0.session_store;

    match store.update_session_state(&session_id, "cancelled").await {
        Ok(()) => (
            axum::http::StatusCode::OK,
            Json(serde_json::json!({
                "status": "cancelled",
                "session_id": session_id,
            })),
        ),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("not found") {
                (
                    axum::http::StatusCode::NOT_FOUND,
                    Json(serde_json::json!({
                        "error": format!("Session not found: {}", session_id),
                    })),
                )
            } else {
                (
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "error": format!("Failed to cancel session: {}", e),
                    })),
                )
            }
        }
    }
}

pub async fn get_transcript(
    state: axum::extract::State<Arc<SessionState>>,
    Path(session_id): Path<String>,
) -> (axum::http::StatusCode, Json<serde_json::Value>) {
    let store = &state.0.session_store;
    let dir = store.session_dir(&session_id);
    let messages_path = dir.join("messages.jsonl");

    if !messages_path.exists() {
        return (
            axum::http::StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": format!("Session not found: {}", session_id),
            })),
        );
    }

    match store.load_messages(&session_id).await {
        Ok(messages) => (
            axum::http::StatusCode::OK,
            Json(serde_json::json!({
                "session_id": session_id,
                "message_count": messages.len(),
                "messages": messages,
            })),
        ),
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": format!("Failed to load messages: {}", e),
            })),
        ),
    }
}
