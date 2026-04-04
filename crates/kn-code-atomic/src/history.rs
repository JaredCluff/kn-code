use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// A snapshot of a file at a point in time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileSnapshot {
    pub path: PathBuf,
    pub content: Vec<u8>,
    pub hash: String,
    pub timestamp: DateTime<Utc>,
    pub session_id: String,
    pub turn_number: u64,
    pub change_type: SnapshotChangeType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotChangeType {
    Created,
    Modified,
    Deleted,
}

/// Diff statistics for a snapshot.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DiffStats {
    pub insertions: usize,
    pub deletions: usize,
}

/// File history tracking system.
///
/// Mirrors Claude Code's fileHistory system:
/// - Snapshot-based file versioning (max 100 snapshots)
/// - Backup files with hash-based naming
/// - Diff stats tracking (insertions/deletions)
/// - SDK mode support via CLAUDE_CODE_ENABLE_SDK_FILE_CHECKPOINTING
pub struct FileHistory {
    pub base_dir: PathBuf,
    pub max_snapshots: usize,
    pub snapshots: Vec<FileSnapshot>,
    pub diff_stats: HashMap<String, DiffStats>,
    pub sdk_checkpointing: bool,
}

impl FileHistory {
    pub fn new(base_dir: PathBuf) -> Self {
        let sdk_checkpointing = std::env::var("KN_CODE_ENABLE_SDK_FILE_CHECKPOINTING").is_ok()
            || std::env::var("CLAUDE_CODE_ENABLE_SDK_FILE_CHECKPOINTING").is_ok();

        Self {
            base_dir,
            max_snapshots: 100,
            snapshots: Vec::new(),
            diff_stats: HashMap::new(),
            sdk_checkpointing,
        }
    }

    /// Track a file edit — creates a snapshot before the edit.
    pub async fn track_edit(
        &mut self,
        path: &Path,
        old_content: &[u8],
        new_content: &[u8],
        session_id: &str,
        turn_number: u64,
    ) -> anyhow::Result<()> {
        if !self.sdk_checkpointing {
            return Ok(());
        }

        let hash = Self::compute_hash(old_content);
        let snapshot_dir = self.base_dir.join(session_id);
        tokio::fs::create_dir_all(&snapshot_dir).await?;

        // Write snapshot file
        let snapshot_path = snapshot_dir.join(format!(
            "{}_{}",
            hash,
            path.file_name().unwrap_or_default().to_string_lossy()
        ));
        tokio::fs::write(&snapshot_path, old_content).await?;

        // Compute diff stats using a simple line-by-line diff
        let old_text = String::from_utf8_lossy(old_content);
        let new_text = String::from_utf8_lossy(new_content);
        let old_lines: Vec<&str> = old_text.lines().collect();
        let new_lines: Vec<&str> = new_text.lines().collect();

        let mut insertions = 0usize;
        let mut deletions = 0usize;
        let mut old_idx = 0;
        let mut new_idx = 0;
        while old_idx < old_lines.len() || new_idx < new_lines.len() {
            if old_idx < old_lines.len()
                && new_idx < new_lines.len()
                && old_lines[old_idx] == new_lines[new_idx]
            {
                old_idx += 1;
                new_idx += 1;
            } else if new_idx < new_lines.len() {
                insertions += 1;
                new_idx += 1;
            } else {
                deletions += 1;
                old_idx += 1;
            }
        }
        let stats = DiffStats {
            insertions,
            deletions,
        };

        self.diff_stats
            .insert(path.to_string_lossy().to_string(), stats);

        let change_type = if old_content.is_empty() {
            SnapshotChangeType::Created
        } else {
            SnapshotChangeType::Modified
        };

        self.snapshots.push(FileSnapshot {
            path: path.to_path_buf(),
            content: old_content.to_vec(),
            hash,
            timestamp: Utc::now(),
            session_id: session_id.to_string(),
            turn_number,
            change_type,
        });

        // Enforce max snapshots — remove oldest
        while self.snapshots.len() > self.max_snapshots {
            self.snapshots.remove(0);
        }

        Ok(())
    }

    /// Restore a file from a snapshot.
    pub async fn restore(&self, snapshot: &FileSnapshot) -> anyhow::Result<()> {
        if let Some(parent) = snapshot.path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(&snapshot.path, &snapshot.content).await?;
        Ok(())
    }

    /// Get the latest snapshot for a file.
    pub fn get_latest(&self, path: &Path) -> Option<&FileSnapshot> {
        self.snapshots.iter().rfind(|s| s.path == path)
    }

    /// Get all snapshots for a session.
    pub fn get_session_snapshots(&self, session_id: &str) -> Vec<&FileSnapshot> {
        self.snapshots
            .iter()
            .filter(|s| s.session_id == session_id)
            .collect()
    }

    /// Get diff stats for a file.
    pub fn get_diff_stats(&self, path: &str) -> Option<&DiffStats> {
        self.diff_stats.get(path)
    }

    /// Clear all snapshots for a session.
    pub fn clear_session(&mut self, session_id: &str) {
        self.snapshots.retain(|s| s.session_id != session_id);
    }

    fn compute_hash(content: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(content);
        format!("{:x}", hasher.finalize())
    }
}
