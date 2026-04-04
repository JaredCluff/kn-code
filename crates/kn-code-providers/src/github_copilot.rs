use crate::traits::*;
use async_trait::async_trait;
use kn_code_auth::Credentials;
use std::collections::HashMap;

pub const GITHUB_COPILOT_API_BASE: &str = "https://api.githubcopilot.com";
pub const GITHUB_COPILOT_TOKEN_URL: &str = "https://api.github.com/copilot_internal/v2/token";

#[derive(Debug)]
pub struct GitHubCopilotProvider {
    pub api_base_url: String,
}

impl Default for GitHubCopilotProvider {
    fn default() -> Self {
        Self {
            api_base_url: GITHUB_COPILOT_API_BASE.to_string(),
        }
    }
}

#[async_trait]
impl Provider for GitHubCopilotProvider {
    fn id(&self) -> &str {
        "github_copilot"
    }

    fn name(&self) -> &str {
        "GitHub Copilot"
    }

    fn auth_methods(&self) -> Vec<String> {
        vec!["oauth".to_string()]
    }

    fn build_client(&self, credentials: &Credentials) -> Result<ProviderClient, ProviderError> {
        let headers = self.required_headers(credentials);
        Ok(ProviderClient {
            base_url: self.api_base_url.clone(),
            headers,
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(600))
                .build()
                .map_err(|e| {
                    ProviderError::Internal(format!("Failed to build HTTP client: {}", e))
                })?,
        })
    }

    async fn chat(
        &self,
        _request: ChatRequest,
        _credentials: &Credentials,
    ) -> Result<ChatResponse, ProviderError> {
        Err(ProviderError::Internal(
            "GitHub Copilot provider not yet implemented".to_string(),
        ))
    }

    async fn chat_stream(
        &self,
        _request: ChatRequest,
        _credentials: &Credentials,
    ) -> Result<ChatStream, ProviderError> {
        Err(ProviderError::Internal(
            "GitHub Copilot streaming not yet implemented".to_string(),
        ))
    }

    fn required_headers(&self, credentials: &Credentials) -> HashMap<String, String> {
        let mut headers = HashMap::new();
        if let Some(token) = credentials.access_token_str() {
            headers.insert("Authorization".to_string(), format!("Bearer {}", token));
        }
        headers
    }

    fn beta_headers(&self) -> Vec<String> {
        vec![]
    }

    fn extra_body_params(&self, _request: &ChatRequest) -> serde_json::Value {
        serde_json::Value::Object(serde_json::Map::new())
    }

    fn parse_error(&self, status: u16, body: &[u8]) -> ProviderError {
        let message = String::from_utf8_lossy(body).to_string();
        ProviderError::ProviderError { status, message }
    }

    async fn verify_credentials(&self, _credentials: &Credentials) -> Result<(), ProviderError> {
        Ok(())
    }

    async fn refresh_credentials(
        &self,
        credentials: &Credentials,
    ) -> Result<Credentials, ProviderError> {
        Ok(credentials.clone())
    }

    async fn list_models(
        &self,
        _credentials: &Credentials,
    ) -> Result<Vec<ModelInfo>, ProviderError> {
        Ok(vec![
            ModelInfo {
                id: "gpt-4o".to_string(),
                provider: "github_copilot".to_string(),
                name: "GPT-4o".to_string(),
                context_window: 128_000,
                max_output_tokens: 4096,
                input_price_per_million: 0.0,
                output_price_per_million: 0.0,
                supports_tools: true,
                supports_vision: true,
                supports_reasoning: false,
            },
            ModelInfo {
                id: "claude-sonnet-4".to_string(),
                provider: "github_copilot".to_string(),
                name: "Claude Sonnet 4".to_string(),
                context_window: 200_000,
                max_output_tokens: 8192,
                input_price_per_million: 0.0,
                output_price_per_million: 0.0,
                supports_tools: true,
                supports_vision: true,
                supports_reasoning: true,
            },
        ])
    }

    fn resolve_model(&self, model_id: &str) -> String {
        model_id.to_string()
    }
}
