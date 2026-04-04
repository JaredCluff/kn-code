use crate::traits::*;
use async_trait::async_trait;
use kn_code_auth::Credentials;
use std::collections::HashMap;

pub const OPENAI_DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";

#[derive(Debug)]
pub struct OpenAIProvider {
    pub base_url: String,
}

impl Default for OpenAIProvider {
    fn default() -> Self {
        Self {
            base_url: OPENAI_DEFAULT_BASE_URL.to_string(),
        }
    }
}

#[async_trait]
impl Provider for OpenAIProvider {
    fn id(&self) -> &str {
        "openai"
    }

    fn name(&self) -> &str {
        "OpenAI"
    }

    fn auth_methods(&self) -> Vec<String> {
        vec!["api_key".to_string(), "oauth".to_string()]
    }

    fn build_client(&self, credentials: &Credentials) -> Result<ProviderClient, ProviderError> {
        let headers = self.required_headers(credentials);
        Ok(ProviderClient {
            base_url: self.base_url.clone(),
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
            "OpenAI provider not yet implemented".to_string(),
        ))
    }

    async fn chat_stream(
        &self,
        _request: ChatRequest,
        _credentials: &Credentials,
    ) -> Result<ChatStream, ProviderError> {
        Err(ProviderError::Internal(
            "OpenAI streaming not yet implemented".to_string(),
        ))
    }

    fn required_headers(&self, credentials: &Credentials) -> HashMap<String, String> {
        let mut headers = HashMap::new();
        if let Some(token) = credentials
            .access_token_str()
            .as_ref()
            .or(credentials.api_key_str().as_ref())
        {
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
                provider: "openai".to_string(),
                name: "GPT-4o".to_string(),
                context_window: 128_000,
                max_output_tokens: 4096,
                input_price_per_million: 2.5,
                output_price_per_million: 10.0,
                supports_tools: true,
                supports_vision: true,
                supports_reasoning: false,
            },
            ModelInfo {
                id: "o1".to_string(),
                provider: "openai".to_string(),
                name: "o1".to_string(),
                context_window: 200_000,
                max_output_tokens: 8192,
                input_price_per_million: 15.0,
                output_price_per_million: 60.0,
                supports_tools: true,
                supports_vision: false,
                supports_reasoning: true,
            },
        ])
    }

    fn resolve_model(&self, model_id: &str) -> String {
        model_id.to_string()
    }
}
