# SPEC 007: Session Management and State

## 1. Overview

Sessions are the fundamental unit of work in kn-code. Each session represents a conversation between a user (or orchestrator) and the LLM, including message history, tool calls, file state, and execution context. Sessions must be persistent, resumable, and portable.

## 2. Session Structure

```rust
pub struct Session {
    pub id: SessionId,              // UUID v4
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub cwd: PathBuf,               // Working directory
    pub model: ModelRef,            // provider/model
    pub variant: Option<String>,    // reasoning effort variant
    pub provider_id: String,        // Which provider to use
    pub state: SessionState,
    pub messages: Vec<Message>,     // Full conversation
    pub tool_calls: Vec<ToolCallRecord>,
    pub usage: UsageTotals,
    pub cost_usd: f64,
    pub turns_completed: u64,
    pub metadata: SessionMetadata,
}

pub enum SessionState {
    Active,
    Paused,
    Completed { exit_code: i32, summary: String },
    Failed { error: String },
    Cancelled,
}

pub struct SessionMetadata {
    pub title: Option<String>,          // Auto-generated from first message
    pub tags: Vec<String>,
    pub parent_session: Option<SessionId>,  // For sub-agents
    pub fork_point: Option<u64>,            // Message index where forked
    pub agent_name: Option<String>,         // Named sub-agent
    pub team_name: Option<String>,          // Agent team
    pub permission_mode: PermissionMode,
    pub max_turns: Option<u64>,
    pub max_cost_usd: Option<f64>,
    pub budget_remaining_usd: Option<f64>,
}
```

## 3. Message Format

```rust
pub enum Message {
    User(UserMessage),
    Assistant(AssistantMessage),
    Tool(ToolMessage),
    System(SystemMessage),
}

pub struct UserMessage {
    pub id: MessageId,
    pub content: Vec<ContentBlock>,
    pub timestamp: DateTime<Utc>,
    pub source: MessageSource,  // user, skill, api, system
}

pub struct AssistantMessage {
    pub id: MessageId,
    pub content: Vec<ContentBlock>,
    pub tool_calls: Vec<ToolCall>,
    pub model: String,
    pub stop_reason: Option<StopReason>,
    pub usage: Option<MessageUsage>,
    pub timestamp: DateTime<Utc>,
}

pub struct ToolMessage {
    pub id: MessageId,
    pub tool_use_id: String,
    pub tool_name: String,
    pub input: serde_json::Value,
    pub output: ToolContent,
    pub duration: Option<Duration>,
    pub timestamp: DateTime<Utc>,
    pub is_error: bool,
}

pub struct SystemMessage {
    pub id: MessageId,
    pub content: String,
    pub subtype: SystemMessageSubtype,
    pub timestamp: DateTime<Utc>,
}

pub enum SystemMessageSubtype {
    Compact,          // Context compaction summary
    SessionStart,     // Session initialization
    Handoff,          // Session handoff from previous run
    Error,            // System error notification
    Notification,     // General notification
}

pub enum ContentBlock {
    Text(String),
    Thinking { text: String, signature: Option<String> },
    Image { base64: String, media_type: String },
    ToolUse { id: String, name: String, input: serde_json::Value },
    ToolResult { id: String, content: ToolContent, is_error: bool },
}

pub enum StopReason {
    EndTurn,
    MaxTokens,
    StopSequence,
    ToolUse,
    Cancelled,
}
```

## 4. Session Persistence

```rust
/// Sessions are stored on disk as JSON files
/// ~/.kn-code/sessions/{session_id}/
///   session.json        — Session metadata and state
///   messages.jsonl      — All messages (append-only)
///   tools.jsonl         — Tool call records
///   staging/            — Atomic staging area
///   journal/            — Write-ahead journal

pub struct SessionStore {
    pub base_dir: PathBuf,  // ~/.kn-code/sessions/
}

impl SessionStore {
    /// Create a new session
    pub async fn create(&self, config: SessionConfig) -> Result<Session> {
        let id = SessionId::new_v4();
        let dir = self.base_dir.join(id.to_string());
        tokio::fs::create_dir_all(&dir).await?;

        let session = Session::new(id, config);
        self.save_session(&session).await?;
        Ok(session)
    }

    /// Load a session from disk
    pub async fn load(&self, id: &SessionId) -> Result<Option<Session>> {
        let dir = self.base_dir.join(id.to_string());
        if !dir.exists() { return Ok(None); }

        let session_json = tokio::fs::read_to_string(dir.join("session.json")).await?;
        let mut session: Session = serde_json::from_str(&session_json)?;

        // Replay messages from JSONL
        let messages_file = dir.join("messages.jsonl");
        if messages_file.exists() {
            let file = tokio::fs::File::open(&messages_file).await?;
            let reader = BufReader::new(file);
            let mut messages = Vec::new();
            let mut lines = reader.lines();
            while let Some(line) = lines.next_line().await? {
                messages.push(serde_json::from_str(&line)?);
            }
            session.messages = messages;
        }

        Ok(Some(session))
    }

    /// Append a message (atomic append to JSONL)
    pub async fn append_message(&self, session_id: &SessionId, message: &Message) -> Result<()> {
        let dir = self.base_dir.join(session_id.to_string());
        let file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("messages.jsonl"))
            .await?;

        let mut writer = BufWriter::new(file);
        writer.write_all(serde_json::to_string(message)?.as_bytes()).await?;
        writer.write_all(b"\n").await?;
        writer.flush().await?;
        Ok(())
    }

    /// Save session metadata
    pub async fn save_session(&self, session: &Session) -> Result<()> {
        let dir = self.base_dir.join(session.id.to_string());
        let json = serde_json::to_string_pretty(session)?;
        // Write to temp file, then atomic rename
        let tmp = dir.join("session.json.tmp");
        tokio::fs::write(&tmp, &json).await?;
        tokio::fs::rename(&tmp, dir.join("session.json")).await?;
        Ok(())
    }

    /// List all sessions
    pub async fn list(&self) -> Result<Vec<SessionSummary>> {
        // Read all session directories
        // Parse session.json for each
        // Sort by updated_at descending
    }

    /// Delete a session
    pub async fn delete(&self, id: &SessionId) -> Result<()> {
        let dir = self.base_dir.join(id.to_string());
        tokio::fs::remove_dir_all(&dir).await?;
        Ok(())
    }
}
```

## 5. Session Resume

```rust
/// When resuming a session:
/// 1. Load session from disk
/// 2. Verify cwd still exists and is accessible
/// 3. Check provider authentication
/// 4. Rebuild tool registry and permission context
/// 5. Reconstruct file state cache
/// 6. Add session handoff message if needed:
///    "This session was resumed. Previous work summary: ..."
/// 7. Continue from last message

pub struct SessionHandoff {
    pub previous_session_id: SessionId,
    pub turns_completed: u64,
    pub summary: String,
    pub last_action: String,
    pub pending_tasks: Vec<String>,
}

impl Session {
    pub fn build_handoff_message(&self) -> SystemMessage {
        let last_assistant = self.messages.iter()
            .rev()
            .find(|m| matches!(m, Message::Assistant(_)));

        SystemMessage {
            content: format!(
                "Session resumed from {}. {} turns completed. Last action: {}",
                self.id,
                self.turns_completed,
                last_assistant.map(|m| m.summary()).unwrap_or("none"),
            ),
            subtype: SystemMessageSubtype::Handoff,
            ..
        }
    }
}
```

## 6. Context Compaction

```rust
/// When session context grows too large, compact it
pub struct Compactor {
    /// Max tokens before compaction triggers
    pub max_tokens: usize,  // default: 180_000

    /// Target tokens after compaction
    pub target_tokens: usize,  // default: 80_000

    /// Compaction strategy
    pub strategy: CompactionStrategy,
}

pub enum CompactionStrategy {
    /// Summarize early messages, keep recent ones intact
    Summarize,
    /// Use provider's native compaction (if available)
    Native,
    /// Extract key facts and decisions, discard conversation
    Extract,
}

impl Compactor {
    pub async fn compact(&self, session: &mut Session, provider: &dyn Provider) -> Result<CompactionResult> {
        match self.strategy {
            CompactionStrategy::Summarize => {
                // 1. Identify messages to compact (oldest first)
                // 2. Build summary prompt
                // 3. Call provider to summarize
                // 4. Replace compacted messages with summary
                // 5. Update token counts
            }
            CompactionStrategy::Native => {
                // Use provider's native compaction API
                // Anthropic: prompt_caching with cache breakpoints
            }
            CompactionStrategy::Extract => {
                // 1. Extract: file changes, decisions, facts, pending tasks
                // 2. Build concise summary
                // 3. Replace all messages with summary + recent context
            }
        }
    }
}

pub struct CompactionResult {
    pub tokens_before: usize,
    pub tokens_after: usize,
    pub messages_before: usize,
    pub messages_after: usize,
    pub summary: String,
}
```

## 7. Budget Enforcement

```rust
pub struct BudgetEnforcer {
    /// Max turns per session
    pub max_turns: Option<u64>,

    /// Max cost per session (USD)
    pub max_cost_usd: Option<f64>,

    /// Max tokens per session
    pub max_tokens: Option<u64>,

    /// Current usage
    pub current_turns: u64,
    pub current_cost_usd: f64,
    pub current_tokens: u64,
}

impl BudgetEnforcer {
    /// Check if another turn is allowed
    pub fn check_budget(&self) -> Result<(), BudgetExceeded> {
        if let Some(max) = self.max_turns {
            if self.current_turns >= max {
                return Err(BudgetExceeded::MaxTurns);
            }
        }
        if let Some(max) = self.max_cost_usd {
            if self.current_cost_usd >= max {
                return Err(BudgetExceeded::MaxCost);
            }
        }
        if let Some(max) = self.max_tokens {
            if self.current_tokens >= max {
                return Err(BudgetExceeded::MaxTokens);
            }
        }
        Ok(())
    }

    /// Get remaining budget
    pub fn remaining(&self) -> BudgetRemaining {
        BudgetRemaining {
            turns: self.max_turns.map(|m| m.saturating_sub(self.current_turns)),
            cost_usd: self.max_cost_usd.map(|m| m - self.current_cost_usd),
            tokens: self.max_tokens.map(|m| m.saturating_sub(self.current_tokens)),
        }
    }
}
```

## 8. Sub-Agent Sessions

```rust
/// Sub-agents are child sessions spawned from a parent session
pub struct SubAgentConfig {
    pub parent_session: SessionId,
    pub description: String,
    pub prompt: String,
    pub model: Option<ModelRef>,
    pub permission_mode: PermissionMode,
    pub cwd: Option<PathBuf>,
    pub isolation: Option<IsolationMode>,
    pub name: Option<String>,
    pub run_in_background: bool,
}

pub enum IsolationMode {
    /// No isolation (same filesystem, same context)
    None,
    /// Git worktree isolation
    Worktree,
    /// Full sandbox isolation
    Sandbox,
}

impl SessionManager {
    /// Spawn a sub-agent
    pub async fn spawn_sub_agent(&self, config: SubAgentConfig) -> Result<SubAgentHandle> {
        // 1. Create new session with parent reference
        // 2. Copy relevant context from parent
        // 3. Set up tool pool based on permission mode
        // 4. Start agent loop (sync or background)
        // 5. Return handle for monitoring
    }

    /// Wait for sub-agent to complete
    pub async fn wait_for_agent(&self, handle: SubAgentHandle) -> Result<AgentResult> {
        // Poll or wait for completion
        // Return final messages, usage, cost
    }

    /// Cancel a running sub-agent
    pub async fn cancel_agent(&self, handle: SubAgentHandle) -> Result<()> {
        // Send abort signal
        // Wait for graceful shutdown
        // Force kill if needed
    }
}
```

## 9. Session Forking

```rust
/// Fork a session at a specific message point
pub async fn fork_session(
    &self,
    session_id: &SessionId,
    fork_point: u64,  // message index
) -> Result<SessionId> {
    // 1. Load original session
    // 2. Create new session with copied metadata
    // 3. Copy messages up to fork_point
    // 4. New session starts from fork_point
    // 5. Original session is unchanged
    // 6. New session has parent_session reference
}
```

## 10. System Prompt Construction

```rust
/// System prompt is built dynamically for each turn
pub struct SystemPromptBuilder {
    pub core_identity: String,        // "You are an AI coding agent..."
    pub custom_instructions: Option<String>,  // AGENTS.md / CLAUDE.md content
    pub tool_descriptions: Vec<String>,
    pub permission_mode_prompt: String,
    pub file_state_context: Option<String>,
    pub memory_prompt: Option<String>,
    pub plugin_prompts: Vec<String>,
    pub skill_prompts: Vec<String>,
}

impl SystemPromptBuilder {
    pub async fn build(&self, context: &ToolContext) -> Vec<SystemBlock> {
        let mut blocks = Vec::new();

        // 1. Core identity (always first, cached)
        blocks.push(SystemBlock {
            content: self.core_identity.clone(),
            cache_control: true,
        });

        // 2. Custom instructions (cached if unchanged)
        if let Some(instructions) = &self.custom_instructions {
            blocks.push(SystemBlock {
                content: instructions.clone(),
                cache_control: true,
            });
        }

        // 3. Tool descriptions (cached if tool set unchanged)
        blocks.push(SystemBlock {
            content: self.build_tool_descriptions(context),
            cache_control: true,
        });

        // 4. Permission mode
        blocks.push(SystemBlock {
            content: self.permission_mode_prompt.clone(),
            cache_control: false,
        });

        // 5. File state context (never cached, changes frequently)
        if let Some(file_state) = &self.file_state_context {
            blocks.push(SystemBlock {
                content: file_state.clone(),
                cache_control: false,
            });
        }

        // 6. Plugin prompts
        for prompt in &self.plugin_prompts {
            blocks.push(SystemBlock {
                content: prompt.clone(),
                cache_control: false,
            });
        }

        blocks
    }
}
```

## 11. Custom Instructions Loading

```rust
/// Load custom instructions from project files
/// Priority order (first match wins):
/// 1. .kn-code/AGENTS.md
/// 2. AGENTS.md
/// 3. .claude/CLAUDE.md  (Claude Code compatibility)
/// 4. CLAUDE.md
/// 5. .cursor/rules/     (Cursor compatibility)
/// 6. .github/copilot-instructions.md

pub async fn load_custom_instructions(cwd: &Path) -> Result<Option<String>> {
    let candidates = [
        cwd.join(".kn-code/AGENTS.md"),
        cwd.join("AGENTS.md"),
        cwd.join(".claude/CLAUDE.md"),
        cwd.join("CLAUDE.md"),
        cwd.join(".cursor/rules"),
        cwd.join(".github/copilot-instructions.md"),
    ];

    for path in &candidates {
        if path.exists() {
            return Ok(Some(tokio::fs::read_to_string(path).await?));
        }
    }

    Ok(None)
}
```

## 12. Session Data Directory

```
~/.kn-code/
├── sessions/
│   ├── {session_id_1}/
│   │   ├── session.json
│   │   ├── messages.jsonl
│   │   ├── tools.jsonl
│   │   └── staging/
│   ├── {session_id_2}/
│   │   └── ...
│   └── ...
├── journal/
│   └── {date}/
│       └── {tx_id}.json
├── plugins/
│   └── {plugin_id}.wasm
├── auth/
│   ├── tokens.json       # Encrypted token store
│   └── oauth/            # OAuth state
├── config/
│   └── kn-code.json      # User configuration
├── cache/
│   ├── file_state/       # File state cache
│   └── models/           # Model discovery cache
└── logs/
    └── {date}.log
```

## 13. Session Export/Import

```rust
/// Export a session for sharing or backup
pub struct SessionExport {
    pub version: u32,
    pub session: Session,
    pub messages: Vec<Message>,
    pub tool_calls: Vec<ToolCallRecord>,
    pub files: HashMap<PathBuf, FileSnapshot>,  // Snapshots of modified files
    pub exported_at: DateTime<Utc>,
}

/// Import a session
pub async fn import_session(export: SessionExport) -> Result<SessionId> {
    // 1. Validate export format
    // 2. Create new session
    // 3. Restore messages and tool calls
    // 4. Optionally restore files (with user confirmation)
    // 5. Return new session ID
}
```
