# SPEC 002: Provider and Authentication System

## 1. Overview

kn-code supports multiple LLM providers through a unified trait-based abstraction. Each provider handles its own authentication (OAuth or API key), request formatting, response parsing, and error handling.

## 2. Provider Trait

```rust
pub trait Provider: Send + Sync + Debug {
    /// Unique provider identifier (e.g., "anthropic", "github_copilot", "openai")
    fn id(&self) -> &str;

    /// Human-readable name
    fn name(&self) -> &str;

    /// Supported auth methods
    fn auth_methods(&self) -> Vec<AuthMethod>;

    /// Build HTTP client with auth headers
    fn build_client(&self, credentials: &Credentials) -> Result<ProviderClient>;

    /// Send chat completion (non-streaming)
    async fn chat(&self, request: ChatRequest, credentials: &Credentials)
        -> Result<ChatResponse>;

    /// Send chat completion (streaming)
    async fn chat_stream(
        &self,
        request: ChatRequest,
        credentials: &Credentials,
    ) -> Result<ChatStream>;

    /// Required HTTP headers for every request
    fn required_headers(&self, credentials: &Credentials) -> HashMap<String, String>;

    /// Beta/preview feature headers
    fn beta_headers(&self) -> Vec<String>;

    /// Extra body parameters (provider-specific)
    fn extra_body_params(&self, request: &ChatRequest) -> serde_json::Value;

    /// Parse provider-specific error responses
    fn parse_error(&self, status: u16, body: &[u8]) -> ProviderError;

    /// Check if credentials are valid (lightweight probe)
    async fn verify_credentials(&self, credentials: &Credentials) -> Result<()>;

    /// Refresh OAuth tokens if applicable
    async fn refresh_credentials(&self, credentials: &Credentials) -> Result<Credentials>;

    /// Discover available models from provider API
    async fn list_models(&self, credentials: &Credentials) -> Result<Vec<ModelInfo>>;

    /// Map provider's model name to internal ID
    fn resolve_model(&self, model_id: &str) -> String;
}
```

## 3. Authentication Methods

### 3.1 Enum

```rust
pub enum AuthMethod {
    /// Static API key from env var, config, or keychain
    ApiKey {
        env_var: String,           // e.g., "ANTHROPIC_API_KEY"
        config_key: String,        // e.g., "providers.anthropic.api_key"
        keychain_service: String,  // e.g., "kn-code-anthropic"
    },
    /// OAuth 2.0 PKCE flow
    OAuth {
        client_id: String,
        authorize_url: String,
        token_url: String,
        scopes: Vec<String>,
        redirect_port: u16,        // localhost callback port
    },
    /// API key obtained via OAuth exchange (Anthropic pattern)
    OAuthDerivedApiKey {
        oauth_client_id: String,
        api_key_url: String,
        token_url: String,
        scopes: Vec<String>,
    },
}
```

### 3.2 Credentials

```rust
pub struct Credentials {
    pub provider_id: String,
    pub auth_type: AuthType,
    pub api_key: Option<Secret<String>>,
    pub access_token: Option<Secret<String>>,
    pub refresh_token: Option<Secret<String>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub account_uuid: Option<String>,
    pub user_email: Option<String>,
    pub organization_uuid: Option<String>,
}

pub enum AuthType {
    ApiKey,
    OAuth,
    OAuthDerivedApiKey,
}
```

## 4. Anthropic Provider

### 4.1 Configuration

```rust
pub struct AnthropicConfig {
    pub base_url: String,              // default: https://api.anthropic.com
    pub client_id: String,             // default: 9d1c250a-e61b-44d9-88ed-5944d1962f5e
    pub authorize_url: String,         // default: https://claude.com/cai/oauth/authorize
    pub token_url: String,             // default: https://platform.claude.com/v1/oauth/token
    pub api_key_url: String,           // default: https://api.anthropic.com/api/oauth/claude_cli/create_api_key
    pub profile_url: String,           // default: https://api.anthropic.com/api/oauth/profile
    pub claude_cli_profile_url: String,// default: https://api.anthropic.com/api/claude_cli_profile
    pub mcp_client_metadata_url: String, // default: https://claude.ai/oauth/claude-code-client-metadata
    pub scopes: Vec<String>,
    pub redirect_port: u16,            // default: dynamic OS-assigned
}

impl Default for AnthropicConfig {
    fn default() -> Self {
        Self {
            base_url: "https://api.anthropic.com".into(),
            client_id: "9d1c250a-e61b-44d9-88ed-5944d1962f5e".into(),
            authorize_url: "https://claude.com/cai/oauth/authorize".into(),
            token_url: "https://platform.claude.com/v1/oauth/token".into(),
            api_key_url: "https://api.anthropic.com/api/oauth/claude_cli/create_api_key".into(),
            profile_url: "https://api.anthropic.com/api/oauth/profile".into(),
            claude_cli_profile_url: "https://api.anthropic.com/api/claude_cli_profile".into(),
            mcp_client_metadata_url: "https://claude.ai/oauth/claude-code-client-metadata".into(),
            scopes: vec![
                "user:profile".into(),
                "user:inference".into(),
                "user:sessions:claude_code".into(),
                "user:mcp_servers".into(),
                "user:file_upload".into(),
            ],
            redirect_port: 0, // OS-assigned
        }
    }
}
```

### 4.2 Required Headers (CRITICAL — must match exactly)

```rust
fn required_headers(&self, credentials: &Credentials) -> HashMap<String, String> {
    let mut headers = HashMap::new();

    match credentials.auth_type {
        AuthType::ApiKey => {
            headers.insert("x-api-key".into(), credentials.api_key.unwrap().expose());
        }
        AuthType::OAuth | AuthType::OAuthDerivedApiKey => {
            headers.insert(
                "Authorization".into(),
                format!("Bearer {}", credentials.access_token.unwrap().expose()),
            );
        }
    }

    // These headers are ALWAYS required
    headers.insert("x-app".into(), "cli".into());
    headers.insert("User-Agent".into(), format!("claude-code/{}", VERSION));
    headers.insert("anthropic-version".into(), "2023-06-01".into());
    headers.insert("anthropic-beta".into(), "claude-code-20250219".into());

    // Session tracking
    if let Some(session_id) = self.current_session_id() {
        headers.insert("X-Claude-Code-Session-Id".into(), session_id);
    }

    // Billing attribution
    let fingerprint = self.compute_fingerprint();
    headers.insert(
        "x-anthropic-billing-header".into(),
        format!("cc_version={VERSION}.{fingerprint}; cc_entrypoint=cli;"),
    );

    headers
}

fn beta_headers(&self) -> Vec<String> {
    vec![
        "claude-code-20250219".into(),
        "interleaved-thinking-2025-05-14".into(),
        "context-1m-2025-08-07".into(),
        "structured-outputs-2025-12-15".into(),
        "web-search-2025-03-05".into(),
        "advanced-tool-use-2025-11-20".into(),
        "effort-2025-11-24".into(),
        "token-efficient-tools-2026-03-28".into(),
        "fast-mode-2026-02-01".into(),
    ]
}
```

### 4.3 OAuth Flow

```
1. Generate PKCE code_verifier (43-128 random bytes, base64url)
2. Compute code_challenge = SHA256(code_verifier), base64url encoded
3. Generate state parameter (CSRF protection)
4. Build authorize URL:
   https://claude.com/cai/oauth/authorize
     ?client_id=9d1c250a-e61b-44d9-88ed-5944d1962f5e
     &redirect_uri=http://localhost:{port}/callback
     &response_type=code
     &scope=user:profile+user:inference+user:sessions:claude_code+user:mcp_servers+user:file_upload
     &code_challenge={challenge}
     &code_challenge_method=S256
     &state={state}
5. Open browser to authorize URL
6. Start localhost HTTP server on {port}/callback
7. Capture authorization_code and state from redirect
8. Validate state matches
9. Exchange code for tokens:
   POST https://platform.claude.com/v1/oauth/token
   {
     "grant_type": "authorization_code",
     "code": "{authorization_code}",
     "redirect_uri": "http://localhost:{port}/callback",
     "client_id": "9d1c250a-e61b-44d9-88ed-5944d1962f5e",
     "code_verifier": "{code_verifier}"
   }
10. Response contains:
    {
      "access_token": "...",
      "refresh_token": "...",
      "expires_in": 3600,
      "scope": "user:profile user:inference user:sessions:claude_code user:mcp_servers user:file_upload"
    }
11. Optionally derive API key:
    POST https://api.anthropic.com/api/oauth/claude_cli/create_api_key
    Authorization: Bearer {access_token}
    anthropic-beta: oauth-2025-04-20
```

### 4.4 Token Refresh

```rust
async fn refresh_credentials(&self, credentials: &Credentials) -> Result<Credentials> {
    let response = self.client
        .post(&self.config.token_url)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", credentials.refresh_token.as_ref().unwrap().expose()),
            ("client_id", &self.config.client_id),
            ("scope", &self.config.scopes.join(" ")),
        ])
        .send()
        .await?;

    let token_response: TokenResponse = response.json().await?;

    Ok(Credentials {
        access_token: Some(Secret::new(token_response.access_token)),
        refresh_token: Some(Secret::new(token_response.refresh_token)),
        expires_at: Some(Utc::now() + Duration::seconds(token_response.expires_in)),
        ..credentials.clone()
    })
}
```

### 4.5 Request Format

```rust
// Anthropic uses a different message format than OpenAI
// Must convert from internal ChatRequest to Anthropic format

pub struct AnthropicChatRequest {
    pub model: String,
    pub max_tokens: u32,
    pub system: Vec<SystemBlock>,      // system prompt blocks
    pub messages: Vec<AnthropicMessage>,
    pub tools: Option<Vec<AnthropicTool>>,
    pub tool_choice: Option<ToolChoice>,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub stream: bool,
    pub thinking: Option<ThinkingConfig>,  // for Claude reasoning
    pub betas: Option<Vec<String>>,
}

// System prompt construction
// Claude Code uses multiple system blocks:
// 1. Core identity: "You are Claude Code, Anthropic's official CLI for Claude."
// 2. Custom instructions (CLAUDE.md / AGENTS.md)
// 3. Tool descriptions
// 4. Permission mode instructions
// 5. File state context
// 6. Memory prompts
```

### 4.6 Response Parsing

```rust
// Streaming response events:
// - message_start
// - content_block_start (text, tool_use, thinking)
// - content_block_delta
// - content_block_stop
// - message_delta (usage, stop_reason)
// - message_stop
// - ping (keepalive)

// Must handle interleaved thinking blocks
// Must handle parallel tool calls
// Must track usage from message_delta
```

### 4.7 Error Handling

```rust
// Key error types to handle:
// - 401: Invalid API key / token expired / token revoked
// - 403: Organization not allowed / access denied
// - 429: Rate limit (check headers: anthropic-ratelimit-tokens-remaining)
// - 529: Overloaded (retry with backoff, fallback model after 3 consecutive)
// - 400: Prompt too long / tool use error / refusal

// Retry strategy:
// - Default max retries: 10
// - Base delay: 500ms with exponential backoff + jitter
// - On 529: retry with backoff
// - On 429: retry with backoff, respect retry-after header
// - On 401/403: attempt token refresh, then retry once
// - After 3 consecutive 529s: fallback to smaller model
```

## 5. GitHub Copilot Provider

### 5.1 Configuration

```rust
pub struct GitHubCopilotConfig {
    pub github_authorize_url: String,    // https://github.com/login/oauth/authorize
    pub github_token_url: String,        // https://github.com/login/oauth/access_token
    pub copilot_token_url: String,       // https://api.github.com/copilot_internal/v2/token
    pub api_base_url: String,            // https://api.githubcopilot.com
    pub client_id: String,               // GitHub OAuth app client ID
    pub client_secret: Option<Secret<String>>,
    pub scopes: Vec<String>,
    pub redirect_port: u16,
}

impl Default for GitHubCopilotConfig {
    fn default() -> Self {
        Self {
            github_authorize_url: "https://github.com/login/oauth/authorize".into(),
            github_token_url: "https://github.com/login/oauth/access_token".into(),
            copilot_token_url: "https://api.github.com/copilot_internal/v2/token".into(),
            api_base_url: "https://api.githubcopilot.com".into(),
            client_id: "Iv1.0".into(),  // Use official GitHub CLI client
            scopes: vec!["read:user".into(), "copilot".into()],
            redirect_port: 0,
        }
    }
}
```

### 5.2 OAuth Flow

```
1. PKCE flow (same as Anthropic)
2. Authorize at GitHub
3. Exchange for GitHub access token
4. Use GitHub token to obtain Copilot API token:
   GET https://api.github.com/copilot_internal/v2/token
   Authorization: Bearer {github_token}
5. Response:
   {
     "token": "ghu_...",
     "expires_at": 1700000000
   }
6. Use Copilot token for chat API calls
```

### 5.3 Required Headers

```rust
fn required_headers(&self, credentials: &Credentials) -> HashMap<String, String> {
    let mut headers = HashMap::new();
    headers.insert(
        "Authorization".into(),
        format!("Bearer {}", credentials.access_token.unwrap().expose()),
    );
    headers.insert("Editor-Version".into(), format!("kn-code/{}", VERSION));
    headers.insert("User-Agent".into(), format!("kn-code/{}", VERSION));
    headers
}
```

### 5.4 API Compatibility

GitHub Copilot's chat API is OpenAI-compatible:
- Endpoint: `POST https://api.githubcopilot.com/chat/completions`
- Request format: OpenAI Chat Completions API
- Supports: gpt-4o, claude-sonnet-4 (via Copilot), o1, etc.
- Streaming: SSE with `data: ` prefixed JSON objects

## 6. OpenAI Provider

### 6.1 Configuration

```rust
pub struct OpenAIConfig {
    pub api_base_url: String,          // https://api.openai.com/v1
    pub auth_method: OpenAIAuthMethod,
}

pub enum OpenAIAuthMethod {
    /// Direct API key
    ApiKey { env_var: String },
    /// OAuth (for ChatGPT Pro/Team/Enterprise)
    OAuth {
        client_id: String,
        authorize_url: String,
        token_url: String,
    },
}
```

### 6.2 Required Headers

```rust
// API Key mode:
headers.insert("Authorization".into(), format!("Bearer {}", api_key));

// OAuth mode:
headers.insert("Authorization".into(), format!("Bearer {}", access_token));

// Common:
headers.insert("OpenAI-Beta".into(), "responses=v1".into());  // if using responses API
```

### 6.3 Model Support

```rust
// Supported models:
// - gpt-4o
// - gpt-4o-mini
// - o1 (with reasoning)
// - o3 (with reasoning)
// - o3-mini
// - gpt-5 (when available)

// Effort mapping for reasoning models:
// minimal -> reasoning_effort: "low"
// low -> reasoning_effort: "low"
// medium -> reasoning_effort: "medium"
// high -> reasoning_effort: "high"
// max -> reasoning_effort: "high" (max not supported by OpenAI)
```

## 7. Generic OpenAI-Compatible Provider

```rust
pub struct CompatibleProvider {
    pub name: String,
    pub base_url: String,
    pub api_key: Option<Secret<String>>,
    pub models: Vec<String>,
    pub headers: HashMap<String, String>,
}

// Any provider that implements:
// POST {base_url}/chat/completions
// with OpenAI-compatible request/response format
//
// Examples:
// - Ollama (localhost)
// - LM Studio
// - vLLM
// - Together AI
// - Groq
// - DeepSeek
// - Any LiteLLM proxy
```

## 8. Token Store

```rust
/// Secure credential storage
/// Backends (in priority order):
/// 1. OS keychain (keyring crate)
/// 2. Encrypted file (age encryption)
/// 3. Plain file (fallback, with strict permissions 0600)
pub trait TokenStore: Send + Sync {
    async fn store(&self, provider_id: &str, credentials: &Credentials) -> Result<()>;
    async fn load(&self, provider_id: &str) -> Result<Option<Credentials>>;
    async fn delete(&self, provider_id: &str) -> Result<()>;
    async fn list_providers(&self) -> Result<Vec<String>>;
}

/// Auto-refresh tokens before expiry
pub struct TokenManager {
    store: Box<dyn TokenStore>,
    providers: HashMap<String, Box<dyn Provider>>,
    refresh_buffer: Duration,  // refresh 5 min before expiry
}

impl TokenManager {
    /// Get valid credentials, refreshing if needed
    pub async fn get_credentials(&self, provider_id: &str) -> Result<Credentials>;

    /// Check if token needs refresh
    pub fn needs_refresh(&self, credentials: &Credentials) -> bool;

    /// Background refresh task
    pub async fn start_refresh_loop(&self);
}
```

## 9. Provider Selection and Resolution

```rust
/// Model identifier format: "provider_id/model_id"
/// Examples:
///   - "anthropic/claude-sonnet-4-5"
///   - "github_copilot/gpt-4o"
///   - "openai/o1"
///   - "ollama/llama3.1:70b"
pub struct ModelRef {
    pub provider_id: String,
    pub model_id: String,
    pub variant: Option<String>,  // e.g., "high", "low" for reasoning effort
}

impl FromStr for ModelRef {
    fn from_str(s: &str) -> Result<Self> {
        let parts: Vec<&str> = s.splitn(2, '/').collect();
        if parts.len() == 2 {
            Ok(Self {
                provider_id: parts[0].into(),
                model_id: parts[1].into(),
                variant: None,
            })
        } else {
            // Try to match against known default provider
            Err(ParseError::MissingProvider)
        }
    }
}
```

## 10. Environment Variables

| Variable | Provider | Purpose |
|----------|----------|---------|
| `ANTHROPIC_API_KEY` | Anthropic | Direct API key |
| `ANTHROPIC_BASE_URL` | Anthropic | Custom API endpoint |
| `ANTHROPIC_MODEL` | Anthropic | Default model |
| `GITHUB_TOKEN` | GitHub Copilot | Personal access token |
| `OPENAI_API_KEY` | OpenAI | Direct API key |
| `OPENAI_BASE_URL` | OpenAI | Custom API endpoint |
| `KN_CODE_DEFAULT_MODEL` | All | Default model (provider/model format) |
| `KN_CODE_AUTH_METHOD` | All | Force auth method: `oauth` or `api_key` |
| `HTTP_PROXY` / `HTTPS_PROXY` | All | Proxy configuration |
| `NO_PROXY` | All | Comma-separated hosts to bypass proxy |

## 11. Auth Flow Decision Tree

```
User starts kn-code
  │
  ├─ Is API key set in env/config?
  │   ├─ Yes → Use API key auth
  │   │   └─ Verify key with lightweight probe
  │   │       ├─ Valid → Proceed
  │   │       └─ Invalid → Error, suggest OAuth
  │   │
  │   └─ No → Check for stored OAuth tokens
  │       ├─ Found → Check expiry
  │       │   ├─ Valid → Use stored token
  │       │   └─ Expired → Attempt refresh
  │       │       ├─ Success → Use refreshed token
  │       │       └─ Failed → Start OAuth flow
  │       │
  │       └─ Not found → Start OAuth flow
  │           ├─ User completes auth in browser
  │           ├─ Tokens stored securely
  │           └─ Proceed
  │
  └─ Multiple providers configured?
      └─ Use provider specified in model ref
```
