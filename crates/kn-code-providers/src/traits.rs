use async_trait::async_trait;
use kn_code_auth::Credentials;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt::Debug;
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub provider: String,
    pub name: String,
    pub context_window: usize,
    pub max_output_tokens: usize,
    pub input_price_per_million: f64,
    pub output_price_per_million: f64,
    pub supports_tools: bool,
    pub supports_vision: bool,
    pub supports_reasoning: bool,
}

#[derive(Debug, Clone)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    pub tools: Option<Vec<ToolDefinition>>,
    pub tool_choice: Option<ToolChoice>,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub max_tokens: Option<u32>,
    pub stream: bool,
    pub system: Option<String>,
    pub variant: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub role: MessageRole,
    pub content: Vec<ContentBlock>,
}

#[derive(Debug, Clone)]
pub enum MessageRole {
    User,
    Assistant,
    Tool,
    System,
}

#[derive(Debug, Clone)]
pub enum ContentBlock {
    Text(String),
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    ToolResult {
        id: String,
        content: String,
        is_error: bool,
    },
    Thinking {
        text: String,
    },
}

#[derive(Debug, Clone)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

#[derive(Debug, Clone)]
pub enum ToolChoice {
    Auto,
    Required,
    None,
    Specific(String),
}

#[derive(Debug, Clone)]
pub struct ChatResponse {
    pub content: Vec<ContentBlock>,
    pub model: String,
    pub stop_reason: Option<String>,
    pub usage: Usage,
}

#[derive(Debug, Clone, Default)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_input_tokens: u64,
}

pub type ChatStream = tokio_stream::wrappers::ReceiverStream<Result<StreamEvent, ProviderError>>;

#[derive(Debug, Clone)]
pub enum StreamEvent {
    Text(String),
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    Thinking(String),
    Usage(Usage),
}

#[derive(Debug, Error)]
pub enum ProviderError {
    #[error("Authentication failed: {0}")]
    AuthFailed(String),
    #[error("Rate limited: {0}")]
    RateLimited(String),
    #[error("Provider error: {status} - {message}")]
    ProviderError { status: u16, message: String },
    #[error("Network error: {0}")]
    Network(#[from] reqwest::Error),
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("Model not found: {0}")]
    ModelNotFound(String),
    #[error("Internal error: {0}")]
    Internal(String),
}

#[derive(Debug, Clone)]
pub struct ProviderClient {
    pub base_url: String,
    pub headers: HashMap<String, String>,
    pub client: reqwest::Client,
}

#[async_trait]
pub trait Provider: Send + Sync + Debug {
    fn id(&self) -> &str;
    fn name(&self) -> &str;
    fn auth_methods(&self) -> Vec<String>;
    fn build_client(&self, credentials: &Credentials) -> Result<ProviderClient, ProviderError>;
    async fn chat(
        &self,
        request: ChatRequest,
        credentials: &Credentials,
    ) -> Result<ChatResponse, ProviderError>;
    async fn chat_stream(
        &self,
        request: ChatRequest,
        credentials: &Credentials,
    ) -> Result<ChatStream, ProviderError>;
    fn required_headers(&self, credentials: &Credentials) -> HashMap<String, String>;
    fn beta_headers(&self) -> Vec<String>;
    fn extra_body_params(&self, request: &ChatRequest) -> serde_json::Value;
    fn parse_error(&self, status: u16, body: &[u8]) -> ProviderError;
    async fn verify_credentials(&self, credentials: &Credentials) -> Result<(), ProviderError>;
    async fn refresh_credentials(
        &self,
        credentials: &Credentials,
    ) -> Result<Credentials, ProviderError>;
    async fn list_models(&self, credentials: &Credentials)
    -> Result<Vec<ModelInfo>, ProviderError>;
    fn resolve_model(&self, model_id: &str) -> String;
}
