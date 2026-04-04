use crate::transaction::{FileTransaction, TransactionId};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Serialize, Deserialize)]
pub struct JournalEntry {
    pub version: u32,
    pub r#type: String,
    pub transaction: FileTransaction,
    pub checksum: String,
}

pub struct Journal {
    pub path: PathBuf,
    pub max_entries: usize,
    pub fsync: bool,
}

impl Journal {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            max_entries: 10000,
            fsync: true,
        }
    }

    pub async fn write(&self, entry: &JournalEntry) -> anyhow::Result<()> {
        if !self.path.exists() {
            tokio::fs::create_dir_all(&self.path).await?;
        }

        let entry_path = self.path.join(format!("{}.journal", entry.transaction.id));
        let data = serde_json::to_vec(entry)?;

        let tmp_path = entry_path.with_extension("journal.tmp");
        tokio::fs::write(&tmp_path, &data).await?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&tmp_path)?.permissions();
            perms.set_mode(0o600);
            std::fs::set_permissions(&tmp_path, perms)?;
        }

        tokio::fs::rename(&tmp_path, &entry_path).await?;

        if self.fsync {
            let file = tokio::fs::File::open(&entry_path).await?;
            file.sync_all().await?;
        }

        Ok(())
    }

    pub async fn mark_committed(&self, tx_id: TransactionId) -> anyhow::Result<()> {
        let entry_path = self.path.join(format!("{}.journal", tx_id));
        if entry_path.exists() {
            let committed_path = entry_path.with_extension("journal.committed");
            tokio::fs::write(&committed_path, b"committed").await?;
        }
        Ok(())
    }

    pub async fn mark_rolled_back(&self, tx_id: TransactionId) -> anyhow::Result<()> {
        let entry_path = self.path.join(format!("{}.journal", tx_id));
        if entry_path.exists() {
            let rolled_back_path = entry_path.with_extension("journal.rolled_back");
            tokio::fs::write(&rolled_back_path, b"rolled_back").await?;
        }
        Ok(())
    }

    pub async fn load_incomplete(&self) -> anyhow::Result<Vec<FileTransaction>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }

        let mut incomplete = Vec::new();
        let mut entries = tokio::fs::read_dir(&self.path).await?;

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("journal") {
                let committed_path = path.with_extension("journal.committed");
                let rolled_back_path = path.with_extension("journal.rolled_back");

                if !committed_path.exists() && !rolled_back_path.exists() {
                    let data = tokio::fs::read(&path).await?;
                    let journal_entry: JournalEntry = serde_json::from_slice(&data)?;
                    incomplete.push(journal_entry.transaction);
                }
            }
        }

        Ok(incomplete)
    }

    pub async fn purge_old(&self, max_age: std::time::Duration) -> anyhow::Result<()> {
        if !self.path.exists() {
            return Ok(());
        }

        let cutoff = std::time::SystemTime::now() - max_age;
        let mut entries = tokio::fs::read_dir(&self.path).await?;

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if (path.extension().and_then(|e| e.to_str()) == Some("journal")
                || path.extension().and_then(|e| e.to_str()) == Some("committed")
                || path.extension().and_then(|e| e.to_str()) == Some("rolled_back"))
                && let Ok(metadata) = tokio::fs::metadata(&path).await
                && let Ok(modified) = metadata.modified()
                && modified < cutoff
            {
                let _ = tokio::fs::remove_file(&path).await;
            }
        }

        Ok(())
    }
}
