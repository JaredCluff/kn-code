# SPEC 005: Atomic Operations and Transactional File Changes

## 1. Overview

Unlike Claude Code, which writes files directly to disk, kn-code implements a write-ahead journal and transaction system. All file modifications are tracked as a transaction that can be committed atomically or rolled back on failure. This prevents partial/corrupted states when:

- The LLM generates multiple file writes and one fails
- The process is killed mid-write
- A tool call errors after some files have been written
- The user cancels a run mid-execution

## 2. Design Principles

1. **All-or-nothing**: A turn's file changes either all apply or none do
2. **Recoverable**: If kn-code crashes, the journal can replay or rollback
3. **Verifiable**: Pre-change file hashes are recorded for integrity
4. **Fast**: Writes go to a staging area first, then atomic rename
5. **Compatible**: File state cache still works, just tracks staged vs committed state

## 3. Transaction Model

```rust
/// A single file change within a transaction
pub struct FileChange {
    pub path: PathBuf,
    pub change_type: ChangeType,
    pub new_content: Vec<u8>,
    pub original_hash: Option<Sha256Hash>,  // Hash before modification
    pub original_mtime: Option<SystemTime>,
    pub permissions: Option<u32>,
}

pub enum ChangeType {
    Create,   // New file
    Update,   // Modify existing
    Delete,   // Remove file
    Rename { from: PathBuf, to: PathBuf },
}

/// A transaction grouping multiple file changes
pub struct FileTransaction {
    pub id: TransactionId,
    pub session_id: String,
    pub turn_number: u64,
    pub changes: Vec<FileChange>,
    pub state: TransactionState,
    pub created_at: DateTime<Utc>,
    pub committed_at: Option<DateTime<Utc>>,
}

pub enum TransactionState {
    Pending,     // Changes staged but not applied
    Committing,  // In-progress commit
    Committed,   // All changes applied
    RolledBack,  // Changes discarded
    Failed,      // Commit failed, partial state possible (requires manual recovery)
}
```

## 4. Write-Ahead Journal

```rust
/// The journal persists transaction metadata before any changes are made
pub struct Journal {
    pub path: PathBuf,           // ~/.kn-code/journal/
    pub max_entries: usize,      // Max journal entries to retain
    pub fsync: bool,             // Force fsync on journal write
}

/// Journal entry (written BEFORE any file changes)
pub struct JournalEntry {
    pub transaction: FileTransaction,
    pub checksum: Sha256Hash,    // Checksum of the entry
}

impl Journal {
    /// Write entry to journal (fsync'd)
    pub async fn write(&self, entry: &JournalEntry) -> Result<()>;

    /// Mark entry as committed
    pub async fn mark_committed(&self, tx_id: TransactionId) -> Result<()>;

    /// Mark entry as rolled back
    pub async fn mark_rolled_back(&self, tx_id: TransactionId) -> Result<()>;

    /// Load incomplete transactions (for recovery)
    pub async fn load_incomplete(&self) -> Result<Vec<FileTransaction>>;

    /// Purge old entries
    pub async fn purge_old(&self, max_age: Duration) -> Result<()>;
}

/// Journal entry format (on disk):
/// {
///   "version": 1,
///   "type": "file_transaction",
///   "transaction": { ... },
///   "checksum": "sha256:..."
/// }
///
/// Recovery:
/// - On startup, scan journal for incomplete transactions
/// - If state == Pending or Committing: offer rollback or replay
/// - If state == Committed: entry is historical, can be purged
```

## 5. Staging Area

```rust
/// Staging area for file changes before commit
pub struct StagingArea {
    pub base_dir: PathBuf,       // ~/.kn-code/staging/{session_id}/
    pub changes: HashMap<PathBuf, StagedChange>,
}

pub struct StagedChange {
    pub original_path: PathBuf,
    pub staged_path: PathBuf,    // Path in staging area
    pub change_type: ChangeType,
    pub original_hash: Option<Sha256Hash>,
}

impl StagingArea {
    /// Stage a file change (write content to staging, don't touch original)
    pub async fn stage(&mut self, change: FileChange) -> Result<()> {
        // 1. Compute hash of original file (if exists)
        // 2. Write new content to staging area
        // 3. Record in changes map
    }

    /// Commit all staged changes atomically
    pub async fn commit(&mut self) -> Result<()> {
        // For each change:
        // 1. Verify original file hasn't changed (hash check)
        // 2. If original changed by external process: FAIL transaction
        // 3. For Create: atomic rename from staging to target
        // 4. For Update: write to temp file, then atomic rename
        // 5. For Delete: rename original to backup, then delete backup after all succeed
        // 6. For Rename: stage both sides, execute as delete + create atomically
    }

    /// Roll back all staged changes (discard staging area)
    pub async fn rollback(&mut self) -> Result<()> {
        // Simply delete the staging area; originals are untouched
        self.changes.clear();
    }
}
```

## 6. Atomic Commit Protocol

```rust
/// Two-phase commit for file changes:
///
/// Phase 1: Prepare
///   1. Write journal entry (all changes recorded)
///   2. Write all new content to staging area
///   3. Verify all originals are unchanged
///   4. Check disk space for all targets
///   5. Check permissions on all target directories
///
/// Phase 2: Commit
///   1. For each file change:
///      a. Write to temp file in target directory (.kn-code-tmp-XXXXXX)
///      b. Atomic rename temp -> target (renameat2 with RENAME_NOREPLACE)
///   2. For deletes:
///      a. Rename target to backup (.kn-code-bak-XXXXXX)
///      b. Delete backup after all operations succeed
///   3. Update journal: mark committed
///   4. Clean up staging area
///
/// On any failure during Phase 2:
///   1. Delete any temp files created
///   2. Rename any backups back to originals
///   3. Update journal: mark failed
///   4. Return error with partial state details
```

## 7. Integration with File Tools

### 7.1 FileWriteTool

```rust
impl Tool for FileWriteTool {
    async fn call(&self, input: Value, context: ToolContext) -> Result<ToolResult> {
        let path = PathBuf::from(input["file_path"].as_str().unwrap());
        let content = input["content"].as_str().unwrap().as_bytes();

        // Compute original hash
        let original_hash = if path.exists() {
            Some(compute_sha256(&path).await?)
        } else {
            None
        };

        let change = FileChange {
            path: path.clone(),
            change_type: if path.exists() { ChangeType::Update } else { ChangeType::Create },
            new_content: content.to_vec(),
            original_hash,
            original_mtime: path.metadata().ok().map(|m| m.modified().unwrap()),
            permissions: None,
        };

        // Stage the change
        context.session.staging.stage(change).await?;

        // Return success immediately (change is staged, not committed)
        Ok(ToolResult {
            content: ToolContent::Text(format!("File staged: {}", path.display())),
            // ...
        })
    }
}
```

### 7.2 FileEditTool

```rust
impl Tool for FileEditTool {
    async fn call(&self, input: Value, context: ToolContext) -> Result<ToolResult> {
        // 1. Read original file (from cache or disk)
        // 2. Apply string replacement
        // 3. Stage the modified content (same as FileWriteTool)
        // 4. Return diff preview
    }
}
```

### 7.3 Transaction Commit Triggers

```rust
/// When are staged changes committed?
///
/// Option 1: Auto-commit at end of each turn (default)
///   - After the LLM finishes its response and all tool calls are done
///   - All staged changes in that turn are committed atomically
///
/// Option 2: Explicit commit via tool
///   - LLM calls a `CommitFiles` tool to commit staged changes
///   - Allows the LLM to review changes before committing
///
/// Option 3: Manual commit via API
///   - Paperclip calls POST /v1/sessions/{id}/commit
///   - Allows human review before commit

pub enum CommitStrategy {
    AutoEndOfTurn,
    ExplicitTool,
    ManualApi,
}
```

### 7.4 CommitFiles Tool

```rust
/// Tool for explicit commit of staged changes
pub struct CommitFilesTool;

impl Tool for CommitFilesTool {
    fn name(&self) -> &str { "CommitFiles" }
    fn description(&self) -> &str { "Commit all staged file changes to disk" }
    fn is_destructive(&self) -> bool { true }

    async fn call(&self, _input: Value, context: ToolContext) -> Result<ToolResult> {
        let tx = context.session.staging.begin_commit().await?;
        match tx.commit().await {
            Ok(_) => Ok(ToolResult {
                content: ToolContent::Text(format!(
                    "Committed {} file changes",
                    tx.changes.len()
                )),
                ..
            }),
            Err(e) => {
                // Auto-rollback on failure
                tx.rollback().await?;
                Err(ToolError::CommitFailed { source: e })
            }
        }
    }
}
```

### 7.5 RollbackFiles Tool

```rust
/// Tool for rolling back staged changes
pub struct RollbackFilesTool;

impl Tool for RollbackFilesTool {
    fn name(&self) -> &str { "RollbackFiles" }
    fn description(&self) -> &str { "Discard all staged file changes" }

    async fn call(&self, _input: Value, context: ToolContext) -> Result<ToolResult> {
        let count = context.session.staging.rollback().await?;
        Ok(ToolResult {
            content: ToolContent::Text(format!("Rolled back {} staged changes", count)),
            ..
        })
    }
}
```

## 8. Crash Recovery

```rust
/// On startup, kn-code checks for incomplete transactions:
pub async fn recover_from_journal(journal: &Journal, staging: &StagingArea) -> Result<()> {
    let incomplete = journal.load_incomplete().await?;

    for tx in incomplete {
        match tx.state {
            TransactionState::Pending => {
                // Changes were staged but never committed
                // Safe to rollback (originals are untouched)
                log::warn!("Recovering incomplete transaction {}", tx.id);
                staging.rollback().await?;
                journal.mark_rolled_back(tx.id).await?;
            }
            TransactionState::Committing => {
                // Commit was in progress, may be partially applied
                // Check each file: if temp file exists, clean up
                // If backup exists, restore original
                log::warn!("Recovering interrupted commit {}", tx.id);
                recover_partial_commit(&tx).await?;
                journal.mark_failed(tx.id).await?;
            }
            _ => {}
        }
    }
}
```

## 9. File State Cache Integration

```rust
/// The file state cache must be aware of staged changes
pub struct FileStateCache {
    entries: HashMap<PathBuf, FileStateEntry>,
    staging: Arc<StagingArea>,
}

impl FileStateCache {
    /// Read file content: check staging first, then disk
    pub async fn read(&self, path: &Path) -> Result<Vec<u8>> {
        if let Some(staged) = self.staging.get_staged(path) {
            return Ok(staged.new_content.clone());
        }
        tokio::fs::read(path).await
    }

    /// Check if file has been modified since last read
    pub async fn is_unchanged(&self, path: &Path) -> bool {
        // Check against staged content hash or disk hash
    }

    /// Invalidate cache entry
    pub fn invalidate(&mut self, path: &Path) {
        self.entries.remove(path);
    }

    /// Invalidate all entries (after commit)
    pub fn invalidate_all(&mut self) {
        self.entries.clear();
    }
}
```

## 10. Concurrent Session Safety

```rust
/// If multiple kn-code sessions work on the same directory:
/// - Each session has its own staging area and journal
/// - Commits verify originals haven't changed (hash check)
/// - If an external process (or another session) modified a file
///   since it was read, the commit fails with a conflict error
///
/// Conflict resolution:
/// 1. Fail the transaction
/// 2. Report which files conflicted
/// 3. Allow the LLM to re-read and retry
```

## 11. Performance Considerations

```rust
/// Staging writes are async and buffered
/// Commit uses atomic rename (fast on same filesystem)
/// Journal writes are fsync'd (slow but necessary for safety)
///
/// Optimization: batch journal writes
/// - Accumulate journal entries in memory
/// - Flush to disk every N entries or every T seconds
/// - On crash, may lose recent entries but not safety
///
/// Optimization: copy-on-write for large files
/// - Don't copy entire file to staging if only small portion changed
/// - Use a diff-based approach for large files
/// - Apply patch at commit time
```

## 12. Configuration

```rust
pub struct AtomicConfig {
    /// Commit strategy
    pub commit_strategy: CommitStrategy,

    /// Max staging area size (bytes)
    pub max_staging_size: u64,  // default: 1 GB

    /// Journal retention period
    pub journal_retention: Duration,  // default: 7 days

    /// Force fsync on journal writes
    pub journal_fsync: bool,  // default: true

    /// Enable atomic operations (can be disabled for performance)
    pub enabled: bool,  // default: true
}
```

## 13. API Endpoints for Atomic Operations

```
GET /v1/sessions/{session_id}/staged
```

Returns all currently staged file changes:
```json
{
    "session_id": "abc123",
    "staged_changes": [
        {
            "path": "src/main.rs",
            "change_type": "update",
            "original_hash": "sha256:...",
            "new_size": 1234,
            "diff_preview": "@@ -10,3 +10,5 @@\n..."
        }
    ]
}
```

```
POST /v1/sessions/{session_id}/commit
POST /v1/sessions/{session_id}/rollback
```
