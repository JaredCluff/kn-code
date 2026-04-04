use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::Response;
use futures::{SinkExt, StreamExt};
use kn_code_session::SessionStore;
use std::sync::Arc;
use tokio::sync::mpsc;

const MAX_WS_MESSAGE_SIZE: usize = 64 * 1024;
const WS_IDLE_TIMEOUT_SECS: u64 = 300;
const MAX_WS_MESSAGES_PER_MINUTE: usize = 120;

pub struct WsState {
    pub session_store: Arc<SessionStore>,
    pub jwt_auth: Option<Arc<crate::middleware::auth::JwtAuth>>,
}

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    state: axum::extract::State<Arc<WsState>>,
) -> Response {
    let state = state.0;
    let jwt_auth = state.jwt_auth.clone();

    if jwt_auth.is_some() {
        ws.on_failed_upgrade(|_| {
            tracing::warn!("WebSocket upgrade failed — authentication required");
        })
        .on_upgrade(move |socket| handle_socket(socket, state, jwt_auth))
    } else {
        ws.on_upgrade(move |socket| handle_socket(socket, state, None))
    }
}

async fn handle_socket(
    socket: WebSocket,
    state: Arc<WsState>,
    _jwt_auth: Option<Arc<crate::middleware::auth::JwtAuth>>,
) {
    let (mut sender, mut receiver) = socket.split();
    let (tx, mut rx) = mpsc::channel::<Message>(100);

    let mut send_task = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if sender.send(msg).await.is_err() {
                break;
            }
        }
    });

    let mut recv_task = tokio::spawn(async move {
        let mut message_count = 0;
        let mut last_reset = std::time::Instant::now();
        let idle_timeout = std::time::Duration::from_secs(WS_IDLE_TIMEOUT_SECS);

        loop {
            match tokio::time::timeout(idle_timeout, receiver.next()).await {
                Ok(Some(Ok(msg))) => {
                    let now = std::time::Instant::now();
                    if now.duration_since(last_reset) > std::time::Duration::from_secs(60) {
                        message_count = 0;
                        last_reset = now;
                    }
                    message_count += 1;
                    if message_count > MAX_WS_MESSAGES_PER_MINUTE {
                        let _ = tx
                            .send(Message::Text(
                                serde_json::json!({
                                    "type": "error",
                                    "message": "Rate limit exceeded",
                                })
                                .to_string()
                                .into(),
                            ))
                            .await;
                        continue;
                    }

                    match msg {
                        Message::Text(text) => {
                            if text.len() > MAX_WS_MESSAGE_SIZE {
                                let _ = tx
                                    .send(Message::Text(
                                        serde_json::json!({
                                            "type": "error",
                                            "message": "Message too large",
                                        })
                                        .to_string()
                                        .into(),
                                    ))
                                    .await;
                                continue;
                            }
                            handle_client_message(&state, &text, &tx).await;
                        }
                        Message::Close(_) => {
                            tracing::info!("WebSocket client disconnected");
                            break;
                        }
                        Message::Ping(data) => {
                            let _ = tx.send(Message::Pong(data)).await;
                        }
                        _ => {}
                    }
                }
                Ok(Some(Err(e))) => {
                    tracing::warn!("WebSocket protocol error: {}", e);
                    break;
                }
                Ok(None) => {
                    tracing::info!("WebSocket connection closed by peer");
                    break;
                }
                Err(_) => {
                    tracing::info!("WebSocket idle timeout exceeded");
                    break;
                }
            }
        }
    });

    tokio::select! {
        _ = (&mut send_task) => {
            recv_task.abort();
        }
        _ = (&mut recv_task) => {
            send_task.abort();
        }
    }

    tracing::info!("WebSocket connection closed");
}

async fn handle_client_message(state: &WsState, text: &str, tx: &mpsc::Sender<Message>) {
    let msg: serde_json::Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(_) => {
            send_error(tx, "Invalid JSON").await;
            return;
        }
    };

    let Some(action) = msg.get("action").and_then(|v| v.as_str()) else {
        send_error(tx, "Missing 'action' field").await;
        return;
    };

    match action {
        "ping" => {
            let _ = tx
                .send(Message::Text(
                    serde_json::json!({
                        "type": "pong",
                        "timestamp": chrono::Utc::now().to_rfc3339(),
                    })
                    .to_string()
                    .into(),
                ))
                .await;
        }
        "session_list" => match list_sessions(&state.session_store).await {
            Ok(sessions) => {
                let _ = tx
                    .send(Message::Text(
                        serde_json::json!({
                            "type": "session_list",
                            "sessions": sessions,
                        })
                        .to_string()
                        .into(),
                    ))
                    .await;
            }
            Err(_) => {
                send_error(tx, "Failed to list sessions").await;
            }
        },
        "session_get" => {
            let Some(session_id) = msg.get("session_id").and_then(|v| v.as_str()) else {
                send_error(tx, "Missing 'session_id' field").await;
                return;
            };

            if !is_valid_session_id(session_id) {
                send_error(tx, "Invalid session_id format").await;
                return;
            }

            match get_session(&state.session_store, session_id).await {
                Ok(session) => {
                    let _ = tx
                        .send(Message::Text(
                            serde_json::json!({
                                "type": "session",
                                "session": session,
                            })
                            .to_string()
                            .into(),
                        ))
                        .await;
                }
                Err(_) => {
                    send_error(tx, "Session not found").await;
                }
            }
        }
        unknown => {
            send_error(tx, &format!("Unknown action: {}", unknown)).await;
        }
    }
}

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

async fn list_sessions(store: &SessionStore) -> anyhow::Result<Vec<serde_json::Value>> {
    if !store.base_dir.exists() {
        return Ok(Vec::new());
    }

    let mut sessions = Vec::new();
    let mut entries = tokio::fs::read_dir(&store.base_dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        let session_dir = entry.path();
        if !session_dir.is_dir() {
            continue;
        }

        let dir_name = entry.file_name();
        let dir_name_str = dir_name.to_string_lossy();
        if !is_valid_session_id(&dir_name_str) {
            continue;
        }

        let session_json = session_dir.join("session.json");
        if session_json.exists() {
            let content = tokio::fs::read_to_string(&session_json).await?;
            let record: serde_json::Value = serde_json::from_str(&content)?;
            sessions.push(record);
        }
    }

    Ok(sessions)
}

async fn get_session(store: &SessionStore, session_id: &str) -> anyhow::Result<serde_json::Value> {
    let safe_dir = store.base_dir.join(session_id);
    let canonical_dir = safe_dir.canonicalize()?;

    if !canonical_dir.starts_with(&store.base_dir) {
        anyhow::bail!("Invalid session path");
    }

    let session_json = canonical_dir.join("session.json");
    if !session_json.exists() {
        anyhow::bail!("Session not found");
    }

    let content = tokio::fs::read_to_string(&session_json).await?;
    let record: serde_json::Value = serde_json::from_str(&content)?;
    Ok(record)
}

async fn send_error(tx: &mpsc::Sender<Message>, message: &str) {
    let _ = tx
        .send(Message::Text(
            serde_json::json!({
                "type": "error",
                "message": message,
            })
            .to_string()
            .into(),
        ))
        .await;
}
