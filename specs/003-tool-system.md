# SPEC 003: Tool System Interface and Implementations

## 1. Overview

The tool system enables the LLM to interact with the external world: execute commands, read/write files, search, fetch URLs, delegate to sub-agents, and more. kn-code's tool system mirrors Claude Code's capabilities while adding atomicity guarantees and WASM plugin extensibility.

## 2. Core Tool Trait

```rust
pub trait Tool: Send + Sync + Debug {
    /// Unique tool name (as presented to the LLM)
    fn name(&self) -> &str;

    /// Alternate names for backwards compatibility
    fn aliases(&self) -> &[&str] { &[] }

    /// Short description shown to the LLM in the tool schema
    fn description(&self) -> &str;

    /// Full system prompt instructions for this tool
    fn prompt(&self) -> &str;

    /// JSON Schema for input validation (using schemars)
    fn input_schema(&self) -> serde_json::Value;

    /// JSON Schema for output validation (optional)
    fn output_schema(&self) -> Option<serde_json::Value> { None }

    /// Whether this tool is currently enabled
    fn is_enabled(&self) -> bool { true }

    /// Whether the tool can safely run concurrently
    fn is_concurrency_safe(&self) -> bool { false }

    /// Whether the tool only reads (no side effects)
    fn is_read_only(&self) -> bool { false }

    /// Whether the tool performs irreversible/destructive operations
    fn is_destructive(&self) -> bool { false }

    /// Maximum result size before persisting to disk
    fn max_result_size_chars(&self) -> usize { 100_000 }

    /// Whether to use strict schema validation on input
    fn strict_schema(&self) -> bool { true }

    /// Check if this tool call is permitted
    async fn check_permission(
        &self,
        input: &serde_json::Value,
        context: &ToolContext,
    ) -> Result<PermissionDecision>;

    /// Validate tool-specific constraints beyond schema
    async fn validate_input(
        &self,
        input: &serde_json::Value,
        context: &ToolContext,
    ) -> Result<ValidationResult>;

    /// Execute the tool
    async fn call(
        &self,
        input: serde_json::Value,
        context: ToolContext,
        can_use_tool: &dyn CanUseTool,
        on_progress: &dyn Fn(ToolProgress),
    ) -> Result<ToolResult>;

    /// Extract file path from input (for file-based tools, used by permission rules)
    fn get_path(&self, input: &serde_json::Value) -> Option<PathBuf> { None }

    /// Compact string for security auto-classifier
    fn to_classifier_input(&self, input: &serde_json::Value) -> String { String::new() }
}
```

## 3. Tool Context

```rust
pub struct ToolContext {
    /// Working directory for the session
    pub cwd: PathBuf,

    /// Current conversation messages
    pub messages: Vec<Message>,

    /// Cache of previously read files (path -> content + metadata)
    pub file_state: Arc<Mutex<FileStateCache>>,

    /// Available tools
    pub tools: Arc<ToolRegistry>,

    /// MCP tool instances
    pub mcp_tools: Arc<Vec<McpToolInstance>>,

    /// Abort controller for cancellation
    pub abort: AbortHandle,

    /// Current tool use ID (for API tracking)
    pub tool_use_id: String,

    /// Permission context (mode, rules)
    pub permission_context: PermissionContext,

    /// Session state
    pub session: Arc<SessionState>,

    /// Agent definitions (for sub-agent tools)
    pub agents: Vec<AgentDefinition>,

    /// Whether this is a non-interactive/headless session
    pub is_headless: bool,

    /// Callback to set tool JSX/UI (no-op in headless)
    pub set_display: Box<dyn Fn(ToolDisplay) + Send>,
}
```

## 4. Tool Result

```rust
pub struct ToolResult {
    /// Primary result content
    pub content: ToolContent,

    /// New messages to inject into the conversation
    pub new_messages: Vec<Message>,

    /// Function to modify tool context for subsequent calls
    pub context_modifier: Option<Box<dyn FnOnce(&mut ToolContext) + Send>>,

    /// Whether the result was persisted to disk
    pub persisted: bool,

    /// Path to persisted result file (if applicable)
    pub persisted_path: Option<PathBuf>,

    /// Structured content for programmatic consumers
    pub structured_content: Option<serde_json::Value>,
}

pub enum ToolContent {
    Text(String),
    Image {
        base64: String,
        media_type: String,
        dimensions: Option<(u32, u32)>,
    },
    Multi(Vec<ContentBlock>),
}

pub enum ContentBlock {
    Text(String),
    Image { base64: String, media_type: String },
    Error { message: String, code: String },
}
```

## 5. Permission System

### 5.1 Permission Modes

```rust
pub enum PermissionMode {
    /// Ask for every destructive operation
    Ask,
    /// Auto-accept edits, ask for bash commands
    AcceptEdits,
    /// Auto-accept everything (headless default)
    Auto,
    /// Plan mode - no modifications allowed
    Plan,
}
```

### 5.2 Permission Rules

```rust
pub struct PermissionContext {
    pub mode: PermissionMode,
    pub additional_working_directories: HashSet<PathBuf>,
    pub always_allow: Vec<ToolRule>,
    pub always_deny: Vec<ToolRule>,
    pub always_ask: Vec<ToolRule>,
    pub source_tracking: HashMap<String, PermissionRuleSource>,
}

pub enum ToolRule {
    /// Exact tool name match: "Bash"
    ToolName(String),
    /// Tool with argument prefix: "Bash(git add:*)"
    ToolWithArgs { tool: String, arg_prefix: String },
    /// MCP server-level: "mcp__server1"
    McpServer(String),
    /// MCP wildcard: "mcp__server1__*"
    McpWildcard(String),
    /// Path-based rule
    Path { tool: String, path_pattern: Glob },
}

pub enum PermissionRuleSource {
    Settings,    // From config file
    CliArg,      // From command line
    Command,     // From skill/command
    Session,     // Session-scoped
}
```

### 5.3 Permission Decision

```rust
pub enum PermissionDecision {
    Allow {
        updated_input: Option<serde_json::Value>,
    },
    Deny {
        message: String,
        suggestions: Vec<PermissionUpdate>,
    },
    Ask {
        message: String,
        suggestions: Vec<PermissionUpdate>,
    },
    Passthrough {
        message: Option<String>,
    },
}

pub enum PermissionUpdate {
    AddAllowRule(ToolRule),
    AddDenyRule(ToolRule),
    AddAlwaysAskRule(ToolRule),
}
```

### 5.4 Bash Permission Logic (from Claude Code analysis)

```rust
/// Bash permissions are more complex than other tools because commands
/// can be composed and wrapped. The permission system must strip safe
/// wrappers and env vars to check the underlying command.

/// Safe environment variables that don't affect permission matching
const SAFE_ENV_VARS: &[&str] = &[
    "NODE_ENV", "LANG", "LC_ALL", "TERM", "COLORTERM",
    "NO_COLOR", "FORCE_COLOR", "TZ",
    "RUST_BACKTRACE", "RUST_LOG",
    "GOEXPERIMENT", "GOOS", "GOARCH", "CGO_ENABLED", "GO111MODULE",
    "PYTHONUNBUFFERED", "PYTHONDONTWRITEBYTECODE",
    "ANTHROPIC_API_KEY",  // passing API key is not a permission escalation
    // ... (full list from Claude Code source)
];

/// Safe command wrappers that don't affect permission matching
const SAFE_WRAPPERS: &[&str] = &[
    "timeout", "time", "nice", "nohup", "stdbuf",
];

/// Commands that are always safe to run
const READ_ONLY_COMMANDS: &[&str] = &[
    "ls", "tree", "du", "stat", "file", "wc",
    "cat", "head", "tail", "less", "more",
    "find", "grep", "rg", "ag", "ack",
    "which", "whereis", "locate",
    "jq", "awk", "cut", "sort", "uniq", "tr",
    "git status", "git log", "git diff", "git branch",
    // ...
];

/// Bash permission check flow:
/// 1. Strip safe env var prefixes from command
/// 2. Strip safe wrapper prefixes iteratively (fixed-point)
/// 3. Check deny rules (on stripped command) -> deny
/// 4. Check ask rules -> ask
/// 5. Check if command is in read-only list -> allow
/// 6. Check path constraints (if file-based command)
/// 7. Check exact allow rules -> allow
/// 8. Check prefix allow rules -> allow
/// 9. Check permission mode
/// 10. If still undecided: passthrough to user (or deny in headless)
```

## 6. Tool Registry

```rust
pub struct ToolRegistry {
    tools: HashMap<String, Box<dyn Tool>>,
    aliases: HashMap<String, String>,  // alias -> canonical name
    mcp_tools: Vec<McpToolInstance>,
}

impl ToolRegistry {
    pub fn register(&mut self, tool: Box<dyn Tool>) {
        let name = tool.name().to_string();
        for alias in tool.aliases() {
            self.aliases.insert(alias.to_string(), name.clone());
        }
        self.tools.insert(name, tool);
    }

    pub fn get(&self, name: &str) -> Option<&dyn Tool> {
        self.tools.get(name)
            .map(|t| t.as_ref())
            .or_else(|| self.aliases.get(name)
                .and_then(|canonical| self.tools.get(canonical))
                .map(|t| t.as_ref()))
    }

    pub fn get_all(&self) -> Vec<&dyn Tool> {
        self.tools.values().map(|t| t.as_ref()).collect()
    }

    pub fn get_enabled(&self) -> Vec<&dyn Tool> {
        self.get_all().into_iter().filter(|t| t.is_enabled()).collect()
    }

    pub fn assemble_tool_pool(&self, permission_context: &PermissionContext) -> Vec<&dyn Tool> {
        let mut tools = self.get_enabled();

        // Filter by blanket deny rules
        tools.retain(|tool| {
            !permission_context.always_deny.iter().any(|rule| {
                tool_matches_rule(tool, rule)
            })
        });

        // Sort by name for prompt-cache stability
        tools.sort_by_key(|t| t.name());

        // Deduplicate (built-in tools win over MCP tools with same name)
        tools.dedup_by_key(|t| t.name());

        tools
    }
}
```

## 7. Tool Execution Pipeline

```rust
/// Full tool execution flow (mirrors Claude Code's checkPermissionsAndCallTool)
pub async fn execute_tool(
    tool: &dyn Tool,
    input: serde_json::Value,
    context: ToolContext,
    can_use_tool: &dyn CanUseTool,
) -> Result<ToolResult, ToolError> {
    // Step 1: Schema validation
    let schema = tool.input_schema();
    if tool.strict_schema() {
        validate_schema(&schema, &input)?;
    }

    // Step 2: Tool-specific validation
    let validation = tool.validate_input(&input, &context).await?;
    if !validation.is_valid {
        return Err(ToolError::ValidationFailed {
            message: validation.message.unwrap_or_default(),
        });
    }

    // Step 3: Pre-tool-use hooks
    let hook_result = run_pre_tool_use_hooks(&input, &context).await?;
    let processed_input = hook_result.updated_input.unwrap_or(input);

    // Step 4: Permission resolution
    let decision = if let Some(hook_decision) = hook_result.permission_decision {
        hook_decision
    } else {
        resolve_permission(tool, &processed_input, &context, can_use_tool).await?
    };

    // Step 5: Check decision
    let PermissionDecision::Allow { updated_input } = decision else {
        return Err(ToolError::PermissionDenied {
            message: decision.deny_message().unwrap_or_default().into(),
        });
    };
    let final_input = updated_input.unwrap_or(processed_input);

    // Step 6: Execute tool
    let result = tool.call(final_input, context, can_use_tool, &|progress| {
        // Stream progress updates to caller
    }).await?;

    // Step 7: Handle large results
    let result = if result.content.size_chars() > tool.max_result_size_chars() {
        persist_result_to_disk(&result, &context.session.temp_dir)?
    } else {
        result
    };

    // Step 8: Post-tool-use hooks
    run_post_tool_use_hooks(&result, &context).await?;

    Ok(result)
}
```

## 8. Built-in Tools

### 8.1 BashTool

```rust
pub struct BashTool {
    sandbox_manager: Arc<SandboxManager>,
}

// Input schema:
{
    "type": "object",
    "properties": {
        "command": { "type": "string", "description": "The command to execute" },
        "timeout": { "type": "integer", "description": "Timeout in milliseconds" },
        "description": { "type": "string", "description": "Human-readable description" },
        "run_in_background": { "type": "boolean", "description": "Run asynchronously" },
        "dangerouslyDisableSandbox": { "type": "boolean", "description": "Disable sandbox" }
    },
    "required": ["command"]
}

// Output:
{
    "stdout": "...",
    "stderr": "...",
    "return_code": 0,
    "interrupted": false,
    "background_task_id": null,
    "persisted_output_path": null,
    "return_code_interpretation": "The command succeeded."
}

// Sandboxing:
// - macOS: seatbelt (sandbox-exec)
// - Linux: firejail or bubblewrap
// - Fallback: no sandbox (with warning)
// - Sandbox can be disabled per-command with dangerouslyDisableSandbox

// Execution:
// - Spawns command via tokio::process::Command
// - Uses bash -c for compound commands
// - Streams stdout/stderr with progress polling
// - Auto-backgrounds on timeout
// - Max output capped, excess persisted to disk
```

### 8.2 FileReadTool

```rust
// Input schema:
{
    "type": "object",
    "properties": {
        "file_path": { "type": "string", "description": "Absolute path to the file" },
        "offset": { "type": "integer", "description": "Line number to start from (1-indexed)" },
        "limit": { "type": "integer", "description": "Number of lines to read" },
        "pages": { "type": "string", "description": "PDF page range (e.g., '1-3,5')" }
    },
    "required": ["file_path"]
}

// Features:
// - Handles text files, images, PDFs, Jupyter notebooks
// - Dedup: returns file_unchanged if same file/range read without modification
// - Blocks dangerous device files: /dev/zero, /dev/random, /dev/urandom, /dev/stdin
// - Token-based content validation
// - max_result_size_chars: Infinity (never persists to disk for text)
// - Image files: returns base64 with dimensions
// - PDF files: returns base64 or splits into pages
```

### 8.3 FileWriteTool

```rust
// Input schema:
{
    "type": "object",
    "properties": {
        "file_path": { "type": "string", "description": "Absolute path (must be absolute)" },
        "content": { "type": "string", "description": "Content to write" }
    },
    "required": ["file_path", "content"]
}

// Validation:
// - File must have been read first (check file_state cache)
// - File must not have been modified since last read (stat comparison)
// - Creates parent directories if needed

// Output:
{
    "type": "create" | "update",
    "file_path": "...",
    "content": "...",
    "structured_patch": [...],
    "original_file": "..." | null
}

// ATOMIC: Uses write-ahead journal (see SPEC 005)
```

### 8.4 FileEditTool

```rust
// Input schema:
{
    "type": "object",
    "properties": {
        "file_path": { "type": "string" },
        "old_string": { "type": "string" },
        "new_string": { "type": "string" },
        "replace_all": { "type": "boolean" }
    },
    "required": ["file_path", "old_string", "new_string"]
}

// Features:
// - Find-and-replace style editing
// - Requires file to be read first
// - Validates old_string exists in file
// - Handles quote normalization
// - Max file size: 1 GiB
// - Generates structured patch/diff
// - Redirects Jupyter notebooks to NotebookEditTool

// ATOMIC: Uses write-ahead journal (see SPEC 005)
```

### 8.5 GlobTool

```rust
// Input schema:
{
    "type": "object",
    "properties": {
        "pattern": { "type": "string", "description": "Glob pattern" },
        "path": { "type": "string", "description": "Directory to search in" }
    },
    "required": ["pattern"]
}

// Uses: globset crate for fast glob matching
// Returns: list of matching file paths
```

### 8.6 GrepTool

```rust
// Input schema:
{
    "type": "object",
    "properties": {
        "pattern": { "type": "string", "description": "Regex pattern" },
        "path": { "type": "string", "description": "File or directory to search in" },
        "output_mode": { "type": "string", "enum": ["content", "files_with_matches", "count"] }
    },
    "required": ["pattern"]
}

// Uses: grep crate or ripgrep library
// Respects .gitignore by default
// Returns: matching lines with file:line:content format
```

### 8.7 WebFetchTool

```rust
// Input schema:
{
    "type": "object",
    "properties": {
        "url": { "type": "string", "description": "URL to fetch" },
        "max_length": { "type": "integer", "description": "Max characters to return" }
    },
    "required": ["url"]
}

// Features:
// - Converts HTML to markdown
// - Handles redirects (max 5)
// - Timeout: 30 seconds
// - Max response size: 1MB
// - User-Agent: Claude-User (claude-code/{version}; +https://support.anthropic.com/)
// - Supports: HTML, plain text, JSON, PDF
```

### 8.8 WebSearchTool

```rust
// Input schema:
{
    "type": "object",
    "properties": {
        "query": { "type": "string", "description": "Search query" },
        "num_results": { "type": "integer", "description": "Number of results (default: 10)" }
    },
    "required": ["query"]
}

// Uses: Provider's built-in web search (Anthropic) or external API
// Returns: search results with title, URL, snippet
```

### 8.9 TodoWriteTool

```rust
// Input schema:
{
    "type": "object",
    "properties": {
        "todos": {
            "type": "array",
            "items": {
                "type": "object",
                "properties": {
                    "content": { "type": "string" },
                    "status": { "type": "string", "enum": ["pending", "in_progress", "completed"] }
                },
                "required": ["content", "status"]
            }
        }
    },
    "required": ["todos"]
}

// Manages a todo list for the current session
// Replaces entire list on each call
// Displayed in session status
```

### 8.10 AgentTool (Sub-agent Spawning)

```rust
// Input schema:
{
    "type": "object",
    "properties": {
        "description": { "type": "string", "description": "What this agent will do" },
        "prompt": { "type": "string", "description": "Instructions for the agent" },
        "subagent_type": { "type": "string", "description": "Agent type" },
        "model": { "type": "string", "enum": ["sonnet", "opus", "haiku"] },
        "run_in_background": { "type": "boolean" },
        "name": { "type": "string", "description": "Named agent for addressing" },
        "team_name": { "type": "string", "description": "Agent team" },
        "mode": { "type": "string", "enum": ["ask", "accept_edits", "auto"] },
        "isolation": { "type": "string", "enum": ["worktree", "remote"] },
        "cwd": { "type": "string" }
    },
    "required": ["description", "prompt"]
}

// Execution modes:
// 1. Sync: Run agent synchronously, collect all messages
// 2. Async/background: Register background agent, return async_launched status
// 3. Named agent: Spawn addressable sub-agent
// 4. Team agent: Spawn visible teammate in tmux (interactive mode)

// Tool pool for workers:
// Workers get their own tool pool based on their permission mode
// Independent of parent's restrictions

// Disallowed tools for sub-agents:
// - TaskOutput, ExitPlanMode, AskUserQuestion, TaskStop
// - WorkflowTool, AgentTool (no recursive spawning for non-internal)
```

### 8.11 SkillTool

```rust
// Input schema:
{
    "type": "object",
    "properties": {
        "skill": { "type": "string", "description": "Skill name (e.g., 'commit', 'review')" },
        "args": { "type": "string", "description": "Optional arguments" }
    },
    "required": ["skill"]
}

// Execution modes:
// 1. Inline: Process skill via prompt expansion, returns new_messages
// 2. Forked: Run skill in isolated sub-agent
// 3. Remote: Load SKILL.md from cloud (internal only)

// Skill permissions:
// - Check deny rules first
// - Auto-allow skills with only safe properties
// - Otherwise ask user with suggestions for allow rules

// Skills can modify:
// - Allowed tools (add to always_allow rules)
// - Model override
// - Effort level
```

### 8.12 MCP Tools

```rust
// MCP tools are dynamically registered from MCP servers
// Each MCP tool has name: "mcp__server_name__tool_name"
// Input schema comes from the MCP server's tool definition
// call() invokes the MCP server's tool endpoint

// MCP resource tools:
// - ListMcpResourcesTool: Lists available MCP resources
// - ReadMcpResourceTool: Reads a specific MCP resource

// MCP tool filtering:
// - Deny rules can match "mcp__server" to block all tools from a server
// - Wildcard: "mcp__server__*" blocks all tools from specific server
```

### 8.13 AskUserQuestionTool

```rust
// Input schema:
{
    "type": "object",
    "properties": {
        "question": { "type": "string" },
        "choices": {
            "type": "array",
            "items": {
                "type": "object",
                "properties": {
                    "label": { "type": "string" },
                    "description": { "type": "string" }
                }
            }
        }
    },
    "required": ["question"]
}

// In headless mode: returns error (no user to ask)
// In interactive mode: shows prompt to user
// In Paperclip mode: returns question to caller for human response
```

## 9. Tool Allow/Deny Lists for Agent Types

```rust
/// Tools disallowed for ALL sub-agents
const ALL_AGENT_DISALLOWED_TOOLS: &[&str] = &[
    "TaskOutput", "ExitPlanMode", "AskUserQuestion",
    "TaskStop", "WorkflowTool",
];

/// Tools allowed for async/background agents
const ASYNC_AGENT_ALLOWED_TOOLS: &[&str] = &[
    "FileRead", "WebSearch", "TodoWrite", "Grep",
    "WebFetch", "Glob", "Bash", "FileEdit",
    "FileWrite", "NotebookEdit", "Skill",
    "ToolSearch",
];

/// Tools allowed for coordinator mode
const COORDINATOR_MODE_ALLOWED_TOOLS: &[&str] = &[
    "Agent", "TaskStop", "SendMessage", "SyntheticOutput",
];
```

## 10. Pre/Post Tool Use Hooks

```rust
pub trait PreToolUseHook: Send + Sync {
    fn name(&self) -> &str;
    async fn run(&self, input: &serde_json::Value, context: &ToolContext)
        -> Result<HookResult>;
}

pub struct HookResult {
    /// Override the tool input
    pub updated_input: Option<serde_json::Value>,
    /// Override the permission decision
    pub permission_decision: Option<PermissionDecision>,
    /// Progress message to display
    pub progress_message: Option<String>,
    /// Prevent tool execution
    pub prevent_execution: bool,
    /// Stop entire session
    pub stop_session: bool,
}

pub trait PostToolUseHook: Send + Sync {
    fn name(&self) -> &str;
    async fn run(&self, result: &ToolResult, context: &ToolContext)
        -> Result<()>;
}

/// Built-in hooks:
/// - Auto-background long-running bash commands
/// - Security classifier for bash commands
/// - File state cache invalidation after writes
/// - LSP server notification after file changes
/// - Session compaction trigger after N tool uses
```
