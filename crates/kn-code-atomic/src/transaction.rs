use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

pub type Sha256Hash = String;
pub type TransactionId = String;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileChange {
    pub path: PathBuf,
    pub change_type: ChangeType,
    pub new_content: Vec<u8>,
    pub original_hash: Option<Sha256Hash>,
    pub original_mtime: Option<u64>,
    pub permissions: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChangeType {
    Create,
    Update,
    Delete,
    Rename { from: PathBuf, to: PathBuf },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileTransaction {
    pub id: TransactionId,
    pub session_id: String,
    pub turn_number: u64,
    pub changes: Vec<FileChange>,
    pub state: TransactionState,
    pub created_at: DateTime<Utc>,
    pub committed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransactionState {
    Pending,
    Committing,
    Committed,
    RolledBack,
    Failed,
}

impl FileTransaction {
    pub fn new(session_id: String, turn_number: u64) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            session_id,
            turn_number,
            changes: Vec::new(),
            state: TransactionState::Pending,
            created_at: Utc::now(),
            committed_at: None,
        }
    }

    pub fn add_change(&mut self, change: FileChange) {
        self.changes.push(change);
    }
}

pub async fn compute_sha256(path: &PathBuf) -> anyhow::Result<Sha256Hash> {
    let content = tokio::fs::read(path).await?;
    let mut hasher = Sha256::new();
    hasher.update(&content);
    let result = hasher.finalize();
    Ok(format!("{:x}", result))
}
