use crate::messages::Message;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRecord {
    pub id: String,
    pub created_at: chrono::DateTime<Utc>,
    pub updated_at: chrono::DateTime<Utc>,
    pub cwd: PathBuf,
    pub model: String,
    pub state: String,
    pub turns_completed: u64,
    pub cost_usd: f64,
}

pub struct SessionStore {
    pub base_dir: PathBuf,
    write_locks: Arc<Mutex<HashMap<String, Arc<Mutex<()>>>>>,
}

impl SessionStore {
    pub fn new(base_dir: PathBuf) -> Self {
        Self {
            base_dir,
            write_locks: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    async fn get_session_lock(&self, session_id: &str) -> Arc<Mutex<()>> {
        let mut locks = self.write_locks.lock().await;
        let lock = locks
            .entry(session_id.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone();

        if locks.len() > 10_000 {
            let stale_keys: Vec<String> = locks
                .iter()
                .filter(|(_, lock)| Arc::strong_count(lock) <= 1)
                .map(|(k, _)| k.clone())
                .take(locks.len() / 2)
                .collect();
            for key in &stale_keys {
                locks.remove(key);
            }
            tracing::warn!(
                remaining = locks.len(),
                pruned = stale_keys.len(),
                "Pruned stale session locks"
            );
        }

        lock
    }

    pub fn session_dir(&self, session_id: &str) -> PathBuf {
        self.base_dir.join(session_id)
    }

    pub async fn create_session(
        &self,
        cwd: PathBuf,
        model: String,
    ) -> anyhow::Result<SessionRecord> {
        let id = Uuid::new_v4().to_string();
        let dir = self.session_dir(&id);
        tokio::fs::create_dir_all(&dir).await?;

        let now = Utc::now();
        let record = SessionRecord {
            id: id.clone(),
            created_at: now,
            updated_at: now,
            cwd,
            model,
            state: "active".to_string(),
            turns_completed: 0,
            cost_usd: 0.0,
        };

        let json = serde_json::to_string_pretty(&record)?;
        let tmp_path = dir.join("session.json.tmp");
        tokio::fs::write(&tmp_path, &json).await?;
        tokio::fs::rename(&tmp_path, dir.join("session.json")).await?;
        Ok(record)
    }

    pub async fn load_session(&self, session_id: &str) -> anyhow::Result<Option<SessionRecord>> {
        let dir = self.session_dir(session_id);
        let path = dir.join("session.json");
        if !path.exists() {
            return Ok(None);
        }
        let content = tokio::fs::read_to_string(&path).await?;
        let record: SessionRecord = serde_json::from_str(&content)?;
        Ok(Some(record))
    }

    const MAX_MESSAGES_FILE_SIZE: u64 = 50 * 1024 * 1024;

    pub async fn append_message(&self, session_id: &str, message: &Message) -> anyhow::Result<()> {
        let lock = self.get_session_lock(session_id).await;
        let _guard = lock.lock().await;
        let dir = self.session_dir(session_id);
        let path = dir.join("messages.jsonl");

        if path.exists() {
            let metadata = tokio::fs::metadata(&path).await?;
            if metadata.len() >= Self::MAX_MESSAGES_FILE_SIZE {
                anyhow::bail!(
                    "Session transcript exceeded {} MB limit. Use compaction or start a new session.",
                    Self::MAX_MESSAGES_FILE_SIZE / 1024 / 1024
                );
            }
        }

        let file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .await?;

        use tokio::io::AsyncWriteExt;
        let mut writer = tokio::io::BufWriter::new(file);
        let line = serde_json::to_string(message)?;
        writer.write_all(line.as_bytes()).await?;
        writer.write_all(b"\n").await?;
        writer.flush().await?;
        Ok(())
    }

    pub async fn load_messages(&self, session_id: &str) -> anyhow::Result<Vec<Message>> {
        let lock = self.get_session_lock(session_id).await;
        let _guard = lock.lock().await;
        let dir = self.session_dir(session_id);
        let path = dir.join("messages.jsonl");
        if !path.exists() {
            return Ok(Vec::new());
        }
        let content = tokio::fs::read_to_string(&path).await?;
        let mut malformed = 0u64;
        let messages = content
            .lines()
            .filter_map(|line| match serde_json::from_str(line) {
                Ok(msg) => Some(msg),
                Err(e) => {
                    malformed += 1;
                    tracing::warn!(
                        session_id,
                        error = %e,
                        "Skipping malformed message line in session transcript"
                    );
                    None
                }
            })
            .collect();
        if malformed > 0 {
            tracing::warn!(
                session_id,
                malformed,
                "Skipped malformed message lines — session history may be incomplete"
            );
        }
        Ok(messages)
    }

    pub async fn update_session_state(&self, session_id: &str, state: &str) -> anyhow::Result<()> {
        let lock = self.get_session_lock(session_id).await;
        let _guard = lock.lock().await;
        let dir = self.session_dir(session_id);
        let session_json = dir.join("session.json");

        if !session_json.exists() {
            anyhow::bail!("Session not found: {}", session_id);
        }

        let content = tokio::fs::read_to_string(&session_json).await?;
        let mut record: SessionRecord = serde_json::from_str(&content)?;
        record.state = state.to_string();
        record.updated_at = chrono::Utc::now();

        let json = serde_json::to_string_pretty(&record)?;
        let tmp_path = dir.join("session.json.tmp");
        tokio::fs::write(&tmp_path, &json).await?;
        tokio::fs::rename(&tmp_path, &session_json).await?;

        Ok(())
    }
}
