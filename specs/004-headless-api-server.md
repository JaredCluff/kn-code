# SPEC 004: Headless API Server

## 1. Overview

The headless API server is the primary interface for kn-code when orchestrated by Paperclip or any external system. It provides REST endpoints and WebSocket streaming for agent invocation, session management, and status monitoring.

## 2. Server Architecture

```rust
// Built with axum (tokio-based HTTP framework)
// Listens on configurable port (default: 3200)
// Supports HTTP/1.1 and HTTP/2

pub struct ServerConfig {
    pub host: String,              // default: "127.0.0.1"
    pub port: u16,                 // default: 3200
    pub auth: AuthMode,
    pub cors: Option<CorsConfig>,
    pub tls: Option<TlsConfig>,
    pub max_concurrent_sessions: usize,  // default: 10
    pub request_timeout: Duration,       // default: 10 minutes
    pub stream_keepalive: Duration,      // default: 30 seconds
}

pub enum AuthMode {
    /// No authentication (loopback only, dev mode)
    None,
    /// JWT tokens (Paperclip integration)
    Jwt { secret: Secret<String>, issuer: String },
    /// Static API key
    ApiKey { key: Secret<String> },
    /// Mutual TLS
    Mtls { ca_cert: PathBuf },
}
```

## 3. REST API Endpoints

### 3.1 Health Check

```
GET /health
```

Response:
```json
{
    "status": "ok",
    "version": "0.1.0",
    "uptime_seconds": 3600,
    "active_sessions": 3,
    "memory_mb": 42
}
```

### 3.2 Run Agent (Primary Endpoint)

```
POST /v1/run
Content-Type: application/json
Authorization: Bearer <jwt_or_api_key>
```

Request:
```json
{
    "prompt": "Fix the bug in src/main.rs where the server crashes on startup",
    "cwd": "/path/to/project",
    "model": "anthropic/claude-sonnet-4-5",
    "variant": "high",
    "session_id": null,
    "permission_mode": "auto",
    "max_turns": 50,
    "timeout_seconds": 600,
    "extra_instructions": "Always run tests after making changes",
    "allowed_tools": null,
    "denied_tools": null,
    "env": {
        "DATABASE_URL": "postgres://..."
    },
    "stream": true
}
```

Response (streaming, SSE or JSONL):
```json
// Event: session started
{"type": "session_start", "session_id": "abc123", "timestamp": "..."}

// Event: tool use
{"type": "tool_use", "tool": "Bash", "input": {"command": "cargo build"}, "timestamp": "..."}

// Event: tool result
{"type": "tool_result", "tool": "Bash", "output": {"stdout": "...", "stderr": "...", "return_code": 0}, "timestamp": "..."}

// Event: assistant text
{"type": "text", "content": "I've fixed the issue by...", "timestamp": "..."}

// Event: run complete
{"type": "run_complete", "exit_code": 0, "summary": "Fixed the crash...", "usage": {"input_tokens": 5000, "output_tokens": 3000, "cached_tokens": 4000}, "cost_usd": 0.042, "session_id": "abc123", "timestamp": "..."}

// Event: error
{"type": "error", "message": "...", "code": "AUTH_FAILED", "timestamp": "..."}
```

Response (non-streaming):
```json
{
    "session_id": "abc123",
    "exit_code": 0,
    "summary": "Fixed the crash by updating the initialization sequence...",
    "messages": [
        {"role": "assistant", "content": "..."},
        {"role": "tool", "tool": "Bash", "content": "..."},
        {"role": "assistant", "content": "..."}
    ],
    "usage": {
        "input_tokens": 5000,
        "output_tokens": 3000,
        "cached_input_tokens": 4000
    },
    "cost_usd": 0.042,
    "duration_seconds": 45,
    "tool_calls": 12
}
```

### 3.3 Resume Session

```
POST /v1/run
{
    "prompt": "Now add error handling",
    "session_id": "abc123",
    "cwd": "/path/to/project",
    ...
}
```

### 3.4 Cancel Run

```
POST /v1/sessions/{session_id}/cancel
```

Response:
```json
{
    "session_id": "abc123",
    "status": "cancelled",
    "exit_code": 130
}
```

### 3.5 Get Session Status

```
GET /v1/sessions/{session_id}
```

Response:
```json
{
    "session_id": "abc123",
    "status": "running",
    "model": "anthropic/claude-sonnet-4-5",
    "cwd": "/path/to/project",
    "started_at": "2026-04-02T10:00:00Z",
    "turns_completed": 5,
    "cost_usd": 0.021,
    "usage": {
        "input_tokens": 10000,
        "output_tokens": 5000,
        "cached_input_tokens": 8000
    }
}
```

### 3.6 List Sessions

```
GET /v1/sessions
```

Response:
```json
{
    "sessions": [
        {
            "session_id": "abc123",
            "status": "completed",
            "model": "anthropic/claude-sonnet-4-5",
            "cwd": "/path/to/project",
            "started_at": "...",
            "completed_at": "...",
            "exit_code": 0,
            "cost_usd": 0.042
        }
    ]
}
```

### 3.7 Get Session Transcript

```
GET /v1/sessions/{session_id}/transcript
```

Response:
```json
{
    "session_id": "abc123",
    "messages": [
        {"role": "user", "content": "Fix the bug..."},
        {"role": "assistant", "content": "..."},
        {"role": "tool", "tool_name": "Bash", "content": "..."},
        {"role": "assistant", "content": "Done."}
    ]
}
```

### 3.8 Compact Session

```
POST /v1/sessions/{session_id}/compact
```

Compacts the session context to reduce token usage. Returns the new message count and token savings.

### 3.9 Provider Management

```
GET /v1/providers
```

Response:
```json
{
    "providers": [
        {
            "id": "anthropic",
            "name": "Anthropic",
            "auth_methods": ["oauth", "api_key"],
            "authenticated": true,
            "models": ["claude-sonnet-4-5", "claude-opus-4-5", "claude-haiku-4-5"]
        },
        {
            "id": "github_copilot",
            "name": "GitHub Copilot",
            "auth_methods": ["oauth"],
            "authenticated": true,
            "models": ["gpt-4o", "claude-sonnet-4", "o1"]
        },
        {
            "id": "openai",
            "name": "OpenAI",
            "auth_methods": ["api_key", "oauth"],
            "authenticated": false,
            "models": []
        }
    ]
}
```

### 3.10 Authenticate Provider

```
POST /v1/providers/{provider_id}/auth
```

Request (OAuth):
```json
{
    "method": "oauth"
}
```

Response:
```json
{
    "auth_url": "https://claude.com/cai/oauth/authorize?client_id=...&redirect_uri=...",
    "callback_url": "http://127.0.0.1:54321/callback"
}
```

Request (API Key):
```json
{
    "method": "api_key",
    "api_key": "sk-..."
}
```

### 3.11 Get Models

```
GET /v1/models
```

Response:
```json
{
    "models": [
        {
            "id": "anthropic/claude-sonnet-4-5",
            "provider": "anthropic",
            "name": "Claude Sonnet 4.5",
            "context_window": 200000,
            "max_output_tokens": 8192,
            "input_price_per_million": 3.0,
            "output_price_per_million": 15.0,
            "supports_tools": true,
            "supports_vision": true,
            "supports_reasoning": true
        }
    ]
}
```

## 4. WebSocket Endpoint

```
WS /v1/ws
```

For real-time bidirectional communication. Client sends JSON commands, server streams events.

Client -> Server:
```json
{"type": "run", "prompt": "...", "cwd": "...", "model": "..."}
{"type": "cancel", "session_id": "..."}
{"type": "pong"}
```

Server -> Client:
```json
{"type": "text", "content": "..."}
{"type": "tool_use", "tool": "Bash", "input": {"command": "..."}}
{"type": "tool_result", "tool": "Bash", "output": {...}}
{"type": "run_complete", "exit_code": 0, ...}
{"type": "error", "message": "..."}
{"type": "ping"}
```

## 5. JSONL Output Format (CLI Compatibility)

For Paperclip's `kn-code run --format json` mode, output is JSONL to stdout:

```json
{"type": "system", "subtype": "init", "session_id": "abc123", "model": "claude-sonnet-4-5"}
{"type": "text", "content": "I'll start by..."}
{"type": "tool_use", "id": "tool_1", "name": "Bash", "input": {"command": "ls"}}
{"type": "tool_result", "id": "tool_1", "output": {"stdout": "...\n", "stderr": "", "return_code": 0}}
{"type": "text", "content": "I see the files..."}
{"type": "result", "subtype": "success", "session_id": "abc123", "usage": {"input_tokens": 5000, "output_tokens": 3000}, "cost_usd": 0.042}
```

This format is directly compatible with Paperclip's existing `opencode-local` adapter parser.

## 6. Authentication Middleware

```rust
/// JWT authentication (from Paperclip)
pub struct JwtAuth {
    secret: Secret<String>,
    issuer: String,
    audience: String,
}

impl JwtAuth {
    pub fn validate(&self, token: &str) -> Result<JwtClaims> {
        // Validate:
        // - Signature (HMAC-SHA256)
        // - Issuer matches
        // - Audience matches
        // - Not expired
        // - Not before time
    }
}

pub struct JwtClaims {
    pub sub: String,          // agent ID
    pub company_id: String,   // Paperclip company ID
    pub run_id: String,       // Paperclip run ID
    pub exp: i64,
    pub iat: i64,
}
```

## 7. Rate Limiting

```rust
pub struct RateLimiter {
    /// Max requests per minute per client
    requests_per_minute: usize,
    /// Max concurrent sessions
    max_concurrent: usize,
    /// Max tokens per session
    max_tokens_per_session: usize,
}

// Headers on response:
// X-RateLimit-Limit: 60
// X-RateLimit-Remaining: 45
// X-RateLimit-Reset: 1700000000
```

## 8. Logging and Observability

```rust
/// Structured logging (tracing crate)
/// Log levels:
/// - TRACE: Token-level streaming details
/// - DEBUG: Tool execution details
/// - INFO: Session start/complete, API calls
/// - WARN: Rate limits, retries, fallbacks
/// - ERROR: Auth failures, tool errors, panics

/// Metrics (optional Prometheus export)
/// - kn_code_sessions_total (counter)
/// - kn_code_sessions_active (gauge)
/// - kn_code_api_requests_total (counter, labeled by provider)
/// - kn_code_api_errors_total (counter, labeled by provider, error_type)
/// - kn_code_tokens_total (counter, labeled by direction: input/output)
/// - kn_code_cost_usd_total (counter)
/// - kn_code_tool_calls_total (counter, labeled by tool_name)
/// - kn_code_session_duration_seconds (histogram)
/// - kn_code_memory_mb (gauge)
```

## 9. Error Responses

```json
{
    "error": {
        "code": "SESSION_NOT_FOUND",
        "message": "Session 'xyz' not found",
        "details": null
    }
}
```

Error codes:
- `AUTH_FAILED` — Invalid or expired credentials
- `SESSION_NOT_FOUND` — Session ID doesn't exist
- `SESSION_BUSY` — Session is already running
- `RATE_LIMITED` — Too many requests
- `PROVIDER_ERROR` — Upstream provider error
- `TOOL_ERROR` — Tool execution failed
- `TIMEOUT` — Request timed out
- `INTERNAL_ERROR` — Server error
- `INVALID_REQUEST` — Bad request body
- `PERMISSION_DENIED` — Tool/file permission denied
- `MODEL_NOT_FOUND` — Requested model not available
- `NOT_AUTHENTICATED` — Provider needs authentication

## 10. CORS Configuration

```rust
pub struct CorsConfig {
    pub allowed_origins: Vec<String>,
    pub allowed_methods: Vec<String>,     // default: GET, POST, OPTIONS
    pub allowed_headers: Vec<String>,     // default: Authorization, Content-Type
    pub max_age: Duration,                // default: 1 hour
}

// For loopback-only (dev mode): allow all origins
// For production: restrict to Paperclip's URL
```

## 11. TLS Configuration

```rust
pub struct TlsConfig {
    pub cert_path: PathBuf,
    pub key_path: PathBuf,
}

// Optional TLS for production deployments
// Default: plaintext HTTP (suitable for loopback/Tailscale)
```

## 12. Graceful Shutdown

```
SIGTERM / SIGINT:
1. Stop accepting new sessions
2. Wait for running sessions to complete (up to 30s grace period)
3. Force-cancel any still-running sessions
4. Persist all session state
5. Exit cleanly
```

## 13. Environment Variables

| Variable | Purpose | Default |
|----------|---------|---------|
| `KN_CODE_SERVER_HOST` | Bind address | `127.0.0.1` |
| `KN_CODE_SERVER_PORT` | Port | `3200` |
| `KN_CODE_AUTH_MODE` | Auth mode: `none`, `jwt`, `api_key` | `none` |
| `KN_CODE_AUTH_SECRET` | JWT secret or API key | — |
| `KN_CODE_MAX_SESSIONS` | Max concurrent sessions | `10` |
| `KN_CODE_REQUEST_TIMEOUT` | Request timeout (seconds) | `600` |
| `KN_CODE_DATA_DIR` | Session data directory | `~/.kn-code/` |
| `KN_CODE_LOG_LEVEL` | Log level | `info` |
| `KN_CODE_LOG_FORMAT` | Log format: `text`, `json` | `text` |
