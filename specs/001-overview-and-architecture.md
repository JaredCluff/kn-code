# SPEC 001: Project Overview and Architecture

## 1. Project Identity

**Name:** kn-code  
**Language:** Rust (edition 2024)  
**License:** MIT  
**Repository:** `gitrepos/kn-code/`

## 2. Purpose

kn-code is a high-performance, headless-first AI coding agent that provides functional equivalence to Claude Code while adding:

1. **Multi-provider support** — Anthropic (OAuth + API key), GitHub Copilot (OAuth), OpenAI (API key + OAuth), and any OpenAI-compatible provider
2. **Headless API server** — Designed to be called via HTTP/gRPC from Paperclip or any orchestrator
3. **Atomic operations** — Transactional file changes with rollback on failure
4. **WASM plugin system** — Sandboxed third-party extensions
5. **Rust-native performance** — Lower memory, faster startup, true async concurrency

## 3. OAuth Identity Requirements

### 3.1 Anthropic OAuth

To work with Anthropic's OAuth subscription flow, kn-code **must** identify itself as Claude Code to Anthropic's servers. This is non-negotiable — Anthropic's OAuth tokens are scoped to the `claude-code` client.

**Required identifiers (from leaked source analysis):**

| Header | Value | Notes |
|--------|-------|-------|
| `User-Agent` | `claude-code/{version}` | Must match exactly |
| `x-app` | `cli` | Required on all API requests |
| `x-anthropic-billing-header` | `cc_version={version}.{fingerprint}; cc_entrypoint=cli;` | Billing attribution |
| Beta header | `claude-code-20250219` | Required for Claude Code feature access |
| OAuth beta | `oauth-2025-04-20` | Required for OAuth token endpoints |
| OAuth Client ID | `9d1c250a-e61b-44d9-88ed-5944d1962f5e` | Production client UUID |

**OAuth endpoints (production):**
- Authorize: `https://claude.com/cai/oauth/authorize`
- Token: `https://platform.claude.com/v1/oauth/token`
- API Key creation: `https://api.anthropic.com/api/oauth/claude_cli/create_api_key`
- Profile: `GET /api/oauth/profile` (bearer token) or `GET /api/claude_cli_profile` (API key)
- MCP Client Metadata: `https://claude.ai/oauth/claude-code-client-metadata`

**OAuth scopes required:**
```
user:profile
user:inference
user:sessions:claude_code
user:mcp_servers
user:file_upload
```

**Token refresh body:**
```json
{
  "grant_type": "refresh_token",
  "refresh_token": "<token>",
  "client_id": "9d1c250a-e61b-44d9-88ed-5944d1962f5e",
  "scope": "user:profile user:inference user:sessions:claude_code user:mcp_servers user:file_upload"
}
```

### 3.2 GitHub Copilot OAuth

GitHub Copilot uses GitHub's OAuth flow. kn-code must:
- Register as a GitHub OAuth App (or use the Copilot CLI client)
- Support PKCE flow
- Use the Copilot API endpoint (`https://api.githubcopilot.com`)
- Identify with appropriate GitHub CLI user-agent

**OAuth flow:**
1. PKCE code verifier/challenge generation
2. Authorize at `https://github.com/login/oauth/authorize`
3. Exchange at `https://github.com/login/oauth/access_token`
4. Use token to obtain Copilot API token via `https://api.github.com/copilot_internal/v2/token`

### 3.3 Provider Abstraction

All providers implement a common trait:

```rust
pub trait Provider: Send + Sync {
    fn name(&self) -> &str;
    fn auth_method(&self) -> AuthMethod;  // OAuth or ApiKey
    async fn authenticate(&self, config: &AuthConfig) -> Result<Credentials>;
    async fn refresh_credentials(&self, creds: &Credentials) -> Result<Credentials>;
    async fn chat_stream(&self, request: ChatRequest, creds: &Credentials) -> Result<ChatStream>;
    async fn chat(&self, request: ChatRequest, creds: &Credentials) -> Result<ChatResponse>;
    fn required_headers(&self, creds: &Credentials) -> HashMap<String, String>;
    fn beta_headers(&self) -> Vec<String>;
}
```

## 4. Architecture Overview

```
kn-code/
├── src/
│   ├── main.rs              # CLI entrypoint (clap)
│   ├── lib.rs               # Library crate root
│   ├── server/              # Headless API server (axum)
│   │   ├── mod.rs
│   │   ├── routes/          # REST API endpoints
│   │   ├── ws.rs            # WebSocket endpoint for streaming
│   │   └── middleware/      # Auth, logging, rate limiting
│   ├── providers/           # LLM provider implementations
│   │   ├── mod.rs
│   │   ├── anthropic.rs     # Anthropic Claude (OAuth + API key)
│   │   ├── github_copilot.rs # GitHub Copilot (OAuth)
│   │   ├── openai.rs        # OpenAI (API key + OAuth)
│   │   └── compatible.rs    # Generic OpenAI-compatible
│   ├── auth/                # Authentication subsystem
│   │   ├── mod.rs
│   │   ├── oauth.rs         # OAuth 2.0 PKCE implementation
│   │   ├── api_key.rs       # API key management
│   │   ├── token_store.rs   # Secure credential storage
│   │   └── pkce.rs          # PKCE code verifier/challenge
│   ├── tools/               # Tool system
│   │   ├── mod.rs
│   │   ├── registry.rs      # Tool registry and dispatch
│   │   ├── bash.rs          # Shell command execution
│   │   ├── file_read.rs     # File reading
│   │   ├── file_write.rs    # File writing (atomic)
│   │   ├── file_edit.rs     # In-place editing (atomic)
│   │   ├── glob.rs          # File pattern matching
│   │   ├── grep.rs          # Content search
│   │   ├── web_fetch.rs     # URL fetching
│   │   ├── web_search.rs    # Web search
│   │   ├── todo_write.rs    # Todo list management
│   │   ├── agent.rs         # Sub-agent spawning
│   │   ├── mcp.rs           # MCP tool integration
│   │   └── skill.rs         # Skill/command system
│   ├── permissions/         # Permission system
│   │   ├── mod.rs
│   │   ├── rules.rs         # Allow/deny/ask rules
│   │   ├── bash_perms.rs    # Bash-specific permissions
│   │   └── sandbox.rs       # Sandboxing (seatbelt/firejail)
│   ├── session/             # Session management
│   │   ├── mod.rs
│   │   ├── manager.rs       # Session lifecycle
│   │   ├── state.rs         # Session state persistence
│   │   ├── compact.rs       # Context compaction
│   │   └── messages.rs      # Message normalization
│   ├── query/               # LLM query engine
│   │   ├── mod.rs
│   │   ├── engine.rs        # Main query loop
│   │   ├── retry.rs         # Retry with backoff
│   │   └── system_prompt.rs # System prompt construction
│   ├── atomic/              # Atomic operations (NEW)
│   │   ├── mod.rs
│   │   ├── transaction.rs   # File change transactions
│   │   ├── journal.rs       # Write-ahead journal
│   │   └── rollback.rs      # Rollback on failure
│   ├── plugins/             # WASM plugin system (NEW)
│   │   ├── mod.rs
│   │   ├── runtime.rs       # WASM runtime (wasmtime)
│   │   ├── host.rs          # Host function bindings
│   │   ├── sandbox.rs       # Plugin capability isolation
│   │   └── lifecycle.rs     # Plugin load/unload
│   ├── config/              # Configuration
│   │   ├── mod.rs
│   │   ├── settings.rs      # User settings
│   │   ├── migrations.rs    # Config version migrations
│   │   └── env.rs           # Environment variable handling
│   └── utils/               # Utilities
│       ├── mod.rs
│       ├── paths.rs         # Path resolution and validation
│       ├── streaming.rs     # Token streaming helpers
│       └── logging.rs       # Structured logging
├── skills/                  # Built-in skills (SKILL.md files)
├── tests/                   # Integration tests
├── benches/                 # Benchmarks
├── Cargo.toml
└── README.md
```

## 5. Execution Modes

### 5.1 Headless/API Mode (Primary)

```bash
kn-code serve --port 3200 --auth jwt
```

- Runs an HTTP server (axum)
- Accepts JSON requests via REST or WebSocket
- Streams responses as SSE or JSONL
- Authenticates via JWT (from Paperclip) or API key
- Session state persisted to disk for resume

### 5.2 CLI Mode (Interactive)

```bash
kn-code
```

- Terminal-based REPL (using `crossterm` + `ratatui`)
- Optional TUI with React-like component tree
- Full interactive experience with permission prompts

### 5.3 Print Mode (Single-shot)

```bash
kn-code run --format json --model anthropic/claude-sonnet-4-5 "Build a REST API"
```

- Single prompt, run to completion
- Output as JSONL to stdout
- Session resume via `--session <id>`
- Compatible with Paperclip's `opencode run --format json` pattern

## 6. Key Design Decisions

### 6.1 Why Rust

- **Memory efficiency** — Claude Code's TypeScript/Node runtime uses 500MB+ idle; Rust target: <50MB
- **Startup time** — Node has 2-3s cold start; Rust: <100ms
- **True async** — Tokio provides proper async I/O without thread pool blocking
- **Safety** — Memory safety without GC pauses during long-running agent sessions
- **Single binary** — No node_modules, no npm install, no runtime dependency

### 6.2 Headless-First Design

Unlike Claude Code which is TUI-first with headless as an afterthought, kn-code is designed as:
1. API server first (Paperclip integration)
2. CLI second (direct human use)
3. TUI optional (nice-to-have)

This means:
- All state is serializable and resumable
- No JSX/React dependencies in core
- Clean separation between execution and presentation
- Every operation has a programmatic interface

### 6.3 Provider Flexibility

Following OpenCode's model of provider abstraction but adding:
- OAuth support for providers that offer it (Anthropic, GitHub Copilot)
- API key support for all providers
- Automatic provider failover (if one is rate-limited, try another)
- Model discovery via provider API (not hardcoded lists)

## 7. Compatibility Matrix

| Feature | Claude Code | kn-code | Notes |
|---------|-------------|---------|-------|
| Anthropic OAuth | Yes | Yes | Must identify as claude-code |
| Anthropic API Key | Yes | Yes | |
| GitHub Copilot OAuth | No | Yes | New |
| OpenAI API Key | No | Yes | New |
| OpenAI OAuth | No | Yes | New |
| Bedrock | Yes | Planned | |
| Vertex AI | Yes | Planned | |
| OpenAI-compatible | No | Yes | Any /v1/chat/completions endpoint |
| MCP servers | Yes | Yes | Compatible protocol |
| Skills system | Yes | Yes | SKILL.md compatible |
| Sub-agents | Yes | Yes | |
| Session resume | Yes | Yes | Cross-provider |
| Atomic file ops | No | Yes | New enhancement |
| WASM plugins | No | Yes | New enhancement |
| Paperclip adapter | Via opencode_local | Yes | Drop-in compatible |
