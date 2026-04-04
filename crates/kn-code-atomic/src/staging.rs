use crate::transaction::{FileChange, Sha256Hash};
use std::collections::HashMap;
use std::path::PathBuf;

pub struct StagedChange {
    pub original_path: PathBuf,
    pub staged_path: PathBuf,
    pub change_type: String,
    pub original_hash: Option<Sha256Hash>,
    pub new_content: Vec<u8>,
}

pub struct StagingArea {
    pub base_dir: PathBuf,
    pub changes: HashMap<PathBuf, StagedChange>,
}

impl StagingArea {
    pub fn new(base_dir: PathBuf) -> Self {
        Self {
            base_dir,
            changes: HashMap::new(),
        }
    }

    pub async fn stage(&mut self, change: FileChange) -> anyhow::Result<()> {
        let file_name = change
            .path
            .file_name()
            .ok_or_else(|| anyhow::anyhow!("Path has no file name component"))?;
        let staged_path = self.base_dir.join(file_name);
        tokio::fs::create_dir_all(&self.base_dir).await?;
        tokio::fs::write(&staged_path, &change.new_content).await?;

        self.changes.insert(
            change.path.clone(),
            StagedChange {
                original_path: change.path,
                staged_path,
                change_type: format!("{:?}", change.change_type),
                original_hash: change.original_hash,
                new_content: change.new_content,
            },
        );
        Ok(())
    }

    pub async fn commit(&mut self) -> anyhow::Result<()> {
        let changes: Vec<_> = self.changes.drain().collect();

        let mut backups: Vec<(PathBuf, Option<PathBuf>)> = Vec::new();
        for (target_path, _) in &changes {
            let backup_path = target_path.with_extension(format!(
                "kn-backup.{}.{}",
                std::process::id(),
                uuid::Uuid::new_v4()
            ));
            let existed = target_path.exists();
            if existed && let Err(e) = tokio::fs::copy(target_path, &backup_path).await {
                for (prev_target, prev_backup) in backups.drain(..) {
                    if let Some(prev_backup) = prev_backup {
                        let _ = tokio::fs::copy(&prev_backup, &prev_target).await;
                        let _ = tokio::fs::remove_file(&prev_backup).await;
                    }
                }
                anyhow::bail!("Failed to backup {}: {}", target_path.display(), e);
            }
            backups.push((
                target_path.clone(),
                if existed { Some(backup_path) } else { None },
            ));
        }

        for (idx, (target_path, staged)) in changes.iter().enumerate() {
            if let Err(e) = tokio::fs::copy(&staged.staged_path, target_path).await {
                for (rollback_idx, (rolled_target, rolled_backup)) in backups.iter().enumerate() {
                    if rollback_idx <= idx {
                        if let Some(backup) = rolled_backup {
                            let _ = tokio::fs::copy(backup, rolled_target).await;
                            let _ = tokio::fs::remove_file(backup).await;
                        } else {
                            let _ = tokio::fs::remove_file(rolled_target).await;
                        }
                    }
                }
                anyhow::bail!(
                    "Commit failed at {}: {}. Rolled back {} change(s).",
                    target_path.display(),
                    e,
                    idx
                );
            }
        }

        for (_, backup) in &backups {
            if let Some(backup_path) = backup {
                let _ = tokio::fs::remove_file(backup_path).await;
            }
        }

        for (_, staged) in &changes {
            let _ = tokio::fs::remove_file(&staged.staged_path).await;
        }

        Ok(())
    }

    pub async fn rollback(&mut self) -> anyhow::Result<usize> {
        let count = self.changes.len();
        let mut errors = Vec::new();
        for (_, staged) in self.changes.drain() {
            if let Err(e) = tokio::fs::remove_file(&staged.staged_path).await {
                errors.push(format!(
                    "Failed to remove {}: {}",
                    staged.staged_path.display(),
                    e
                ));
            }
        }
        if !errors.is_empty() {
            tracing::warn!("Rollback had {} errors: {:?}", errors.len(), errors);
        }
        Ok(count)
    }

    pub fn get_staged(&self, path: &PathBuf) -> Option<&StagedChange> {
        self.changes.get(path)
    }
}
