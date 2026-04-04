use crate::traits::*;
use async_trait::async_trait;
use kn_code_auth::Credentials;
use std::collections::HashMap;

#[derive(Debug)]
pub struct CompatibleProvider {
    pub name: String,
    pub base_url: String,
    pub models: Vec<String>,
}

impl CompatibleProvider {
    pub fn new(name: String, base_url: String, models: Vec<String>) -> Self {
        Self {
            name,
            base_url,
            models,
        }
    }
}

#[async_trait]
impl Provider for CompatibleProvider {
    fn id(&self) -> &str {
        &self.name
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn auth_methods(&self) -> Vec<String> {
        vec!["api_key".to_string()]
    }

    fn build_client(&self, credentials: &Credentials) -> Result<ProviderClient, ProviderError> {
        let headers = self.required_headers(credentials);
        Ok(ProviderClient {
            base_url: self.base_url.clone(),
            headers,
            client: reqwest::Client::new(),
        })
    }

    async fn chat(
        &self,
        _request: ChatRequest,
        _credentials: &Credentials,
    ) -> Result<ChatResponse, ProviderError> {
        Err(ProviderError::Internal(
            "Compatible provider chat not yet implemented".to_string(),
        ))
    }

    async fn chat_stream(
        &self,
        _request: ChatRequest,
        _credentials: &Credentials,
    ) -> Result<ChatStream, ProviderError> {
        Err(ProviderError::Internal(
            "Compatible provider streaming not yet implemented".to_string(),
        ))
    }

    fn required_headers(&self, credentials: &Credentials) -> HashMap<String, String> {
        let mut headers = HashMap::new();
        if let Some(key) = credentials.api_key_str() {
            headers.insert("Authorization".to_string(), format!("Bearer {}", key));
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
        Ok(self
            .models
            .iter()
            .map(|m| ModelInfo {
                id: m.clone(),
                provider: self.name.clone(),
                name: m.clone(),
                context_window: 128_000,
                max_output_tokens: 4096,
                input_price_per_million: 0.0,
                output_price_per_million: 0.0,
                supports_tools: true,
                supports_vision: false,
                supports_reasoning: false,
            })
            .collect())
    }

    fn resolve_model(&self, model_id: &str) -> String {
        model_id.to_string()
    }
}
