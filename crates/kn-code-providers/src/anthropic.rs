use crate::traits::*;
use async_trait::async_trait;
use futures::StreamExt;
use kn_code_auth::{AuthType, Credentials};
use std::collections::HashMap;
use std::time::Duration;
use tokio::time::sleep;
use tracing;

pub const ANTHROPIC_CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
pub const ANTHROPIC_AUTHORIZE_URL: &str = "https://claude.com/cai/oauth/authorize";
pub const ANTHROPIC_TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
pub const ANTHROPIC_API_KEY_URL: &str =
    "https://api.anthropic.com/api/oauth/claude_cli/create_api_key";
pub const ANTHROPIC_PROFILE_URL: &str = "https://api.anthropic.com/api/oauth/profile";
pub const ANTHROPIC_DEFAULT_BASE_URL: &str = "https://api.anthropic.com";
pub const ANTHROPIC_API_VERSION: &str = "2023-06-01";

#[derive(Debug)]
pub struct AnthropicProvider {
    pub base_url: String,
    pub max_retries: u32,
    pub retry_delay_ms: u64,
}

impl Default for AnthropicProvider {
    fn default() -> Self {
        Self {
            base_url: ANTHROPIC_DEFAULT_BASE_URL.to_string(),
            max_retries: 3,
            retry_delay_ms: 1000,
        }
    }
}

impl AnthropicProvider {
    pub fn new(base_url: Option<String>) -> Self {
        Self {
            base_url: base_url.unwrap_or_else(|| ANTHROPIC_DEFAULT_BASE_URL.to_string()),
            ..Default::default()
        }
    }

    pub fn with_retries(mut self, max_retries: u32, retry_delay_ms: u64) -> Self {
        self.max_retries = max_retries;
        self.retry_delay_ms = retry_delay_ms;
        self
    }

    async fn with_retry<T, F, Fut>(&self, mut f: F) -> Result<T, ProviderError>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Result<T, ProviderError>>,
    {
        let mut last_error = None;
        for attempt in 0..=self.max_retries {
            match f().await {
                Ok(result) => return Ok(result),
                Err(e) => {
                    let is_retryable = matches!(
                        &e,
                        ProviderError::RateLimited(_) | ProviderError::Network(_)
                    );
                    if !is_retryable || attempt == self.max_retries {
                        return Err(e);
                    }
                    last_error = Some(e);
                    let delay = Duration::from_millis(self.retry_delay_ms * 2u64.pow(attempt));
                    tracing::warn!(
                        "Retryable error (attempt {}/{}): {:?}",
                        attempt + 1,
                        self.max_retries + 1,
                        last_error
                    );
                    sleep(delay).await;
                }
            }
        }
        Err(last_error.unwrap_or_else(|| ProviderError::Internal("Unknown error".to_string())))
    }

    fn build_messages(&self, request: &ChatRequest) -> (Vec<serde_json::Value>, Vec<String>) {
        let mut messages = Vec::new();
        let mut system_texts = Vec::new();

        if let Some(system) = &request.system {
            system_texts.push(system.clone());
        }

        for msg in &request.messages {
            match &msg.role {
                MessageRole::System => {
                    for block in &msg.content {
                        if let ContentBlock::Text(t) = block {
                            system_texts.push(t.clone());
                        }
                    }
                }
                MessageRole::User => {
                    messages.push(serde_json::json!({
                        "role": "user",
                        "content": msg.content.iter().map(|c| match c {
                            ContentBlock::Text(t) => serde_json::json!({"type": "text", "text": t}),
                            ContentBlock::ToolResult { id, content, is_error } => serde_json::json!({
                                "type": "tool_result",
                                "tool_use_id": id,
                                "content": content,
                                "is_error": is_error,
                            }),
                            _ => serde_json::json!({"type": "text", "text": ""}),
                        }).collect::<Vec<_>>(),
                    }));
                }
                MessageRole::Assistant => {
                    let mut content = Vec::new();
                    for block in &msg.content {
                        match block {
                            ContentBlock::Text(t) => {
                                content.push(serde_json::json!({"type": "text", "text": t}))
                            }
                            ContentBlock::ToolUse { id, name, input } => {
                                content.push(serde_json::json!({
                                    "type": "tool_use",
                                    "id": id,
                                    "name": name,
                                    "input": input,
                                }))
                            }
                            ContentBlock::Thinking { text } => content.push(serde_json::json!({
                                "type": "thinking",
                                "thinking": text,
                            })),
                            _ => {}
                        }
                    }
                    messages.push(serde_json::json!({
                        "role": "assistant",
                        "content": content,
                    }));
                }
                MessageRole::Tool => {
                    messages.push(serde_json::json!({
                        "role": "user",
                        "content": msg.content.iter().filter_map(|c| {
                            if let ContentBlock::ToolResult { id, content, is_error } = c {
                                Some(serde_json::json!({
                                    "type": "tool_result",
                                    "tool_use_id": id,
                                    "content": content,
                                    "is_error": is_error,
                                }))
                            } else {
                                None
                            }
                        }).collect::<Vec<_>>(),
                    }));
                }
            }
        }

        (messages, system_texts)
    }

    fn build_tools(&self, tools: &[ToolDefinition]) -> Vec<serde_json::Value> {
        tools
            .iter()
            .map(|t| {
                serde_json::json!({
                    "name": t.name,
                    "description": t.description,
                    "input_schema": t.input_schema,
                })
            })
            .collect()
    }

    fn parse_response(&self, body: &str) -> Result<ChatResponse, ProviderError> {
        let json: serde_json::Value =
            serde_json::from_str(body).map_err(ProviderError::Serialization)?;

        let content_blocks = json
            .get("content")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let mut content = Vec::new();
        for block in &content_blocks {
            match block.get("type").and_then(|v| v.as_str()) {
                Some("text") => {
                    if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                        content.push(ContentBlock::Text(text.to_string()));
                    }
                }
                Some("tool_use") => {
                    let id = block
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let name = block
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let input = block.get("input").cloned().unwrap_or(serde_json::json!({}));
                    content.push(ContentBlock::ToolUse { id, name, input });
                }
                Some("thinking") => {
                    if let Some(text) = block.get("thinking").and_then(|v| v.as_str()) {
                        content.push(ContentBlock::Thinking {
                            text: text.to_string(),
                        });
                    }
                }
                _ => {}
            }
        }

        let usage = json
            .get("usage")
            .map(|u| Usage {
                input_tokens: u.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
                output_tokens: u.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
                cached_input_tokens: u
                    .get("cache_read_input_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0),
            })
            .unwrap_or_default();

        let stop_reason = json
            .get("stop_reason")
            .and_then(|v| v.as_str())
            .map(String::from);

        let model = json
            .get("model")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        Ok(ChatResponse {
            content,
            model,
            stop_reason,
            usage,
        })
    }

    fn parse_stream_event(lines: &[String]) -> Option<Result<StreamEvent, ProviderError>> {
        let data_lines: Vec<&str> = lines
            .iter()
            .filter_map(|l| l.strip_prefix("data: "))
            .collect();

        if data_lines.is_empty() {
            return None;
        }

        let data = data_lines.join("\n");
        if data == "[DONE]" {
            return None;
        }

        let json: serde_json::Value = match serde_json::from_str(&data) {
            Ok(v) => v,
            Err(e) => return Some(Err(ProviderError::Serialization(e))),
        };

        let event_type = json.get("type").and_then(|v| v.as_str())?;

        match event_type {
            "content_block_delta" => {
                let delta = json.get("delta")?;
                match delta.get("type").and_then(|v| v.as_str())? {
                    "text_delta" => {
                        let text = delta.get("text").and_then(|v| v.as_str())?;
                        Some(Ok(StreamEvent::Text(text.to_string())))
                    }
                    "thinking_delta" => {
                        let text = delta.get("thinking").and_then(|v| v.as_str())?;
                        Some(Ok(StreamEvent::Thinking(text.to_string())))
                    }
                    "input_json_delta" => {
                        // Accumulate tool input — handled at higher level
                        None
                    }
                    _ => None,
                }
            }
            "content_block_start" => {
                let block = json.get("content_block")?;
                match block.get("type").and_then(|v| v.as_str())? {
                    "tool_use" => {
                        let id = block.get("id").and_then(|v| v.as_str())?.to_string();
                        let name = block.get("name").and_then(|v| v.as_str())?.to_string();
                        let input = block.get("input").cloned().unwrap_or(serde_json::json!({}));
                        Some(Ok(StreamEvent::ToolUse { id, name, input }))
                    }
                    _ => None,
                }
            }
            "message_delta" => {
                if let Some(usage) = json.get("usage") {
                    let event_usage = Usage {
                        input_tokens: usage
                            .get("input_tokens")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0),
                        output_tokens: usage
                            .get("output_tokens")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0),
                        cached_input_tokens: 0,
                    };
                    Some(Ok(StreamEvent::Usage(event_usage)))
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}

#[async_trait]
impl Provider for AnthropicProvider {
    fn id(&self) -> &str {
        "anthropic"
    }
    fn name(&self) -> &str {
        "Anthropic"
    }

    fn auth_methods(&self) -> Vec<String> {
        vec!["oauth".to_string(), "api_key".to_string()]
    }

    fn build_client(&self, credentials: &Credentials) -> Result<ProviderClient, ProviderError> {
        let headers = self.required_headers(credentials);
        Ok(ProviderClient {
            base_url: self.base_url.clone(),
            headers,
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(600))
                .build()
                .map_err(ProviderError::Network)?,
        })
    }

    async fn chat(
        &self,
        request: ChatRequest,
        credentials: &Credentials,
    ) -> Result<ChatResponse, ProviderError> {
        let provider = self.build_client(credentials)?;
        let (messages, system_texts) = self.build_messages(&request);
        let tools = request.tools.as_ref().map(|t| self.build_tools(t));

        let mut body = serde_json::json!({
            "model": request.model,
            "messages": messages,
            "max_tokens": request.max_tokens.unwrap_or(8192),
        });

        if let Some(tools) = tools {
            body["tools"] = serde_json::Value::Array(tools);
        }

        if !system_texts.is_empty() {
            body["system"] = serde_json::json!([{
                "type": "text",
                "text": system_texts.join("\n"),
                "cache_control": { "type": "ephemeral" }
            }]);
        }

        if let Some(_effort) = &request.variant {
            // Anthropic effort mode — set via x-anthropic-effort header
        }

        if let Some(temp) = request.temperature {
            body["temperature"] =
                serde_json::Value::Number(serde_json::Number::from_f64(temp as f64).unwrap());
        }

        let extra = self.extra_body_params(&request);
        if let Some(obj) = extra.as_object()
            && let Some(body_obj) = body.as_object_mut()
        {
            for (k, v) in obj {
                body_obj.insert(k.clone(), v.clone());
            }
        }

        let url = format!("{}/v1/messages", provider.base_url);

        self.with_retry(|| async {
            let response =
                provider
                    .client
                    .post(&url)
                    .headers((&provider.headers).try_into().map_err(|e| {
                        ProviderError::Internal(format!("Invalid headers: {:?}", e))
                    })?)
                    .json(&body)
                    .send()
                    .await
                    .map_err(ProviderError::Network)?;

            let status = response.status().as_u16();
            let is_success = response.status().is_success();
            let body_bytes = response.bytes().await.map_err(ProviderError::Network)?;

            if !is_success {
                if status == 429 {
                    return Err(ProviderError::RateLimited(
                        String::from_utf8_lossy(&body_bytes).to_string(),
                    ));
                }
                return Err(self.parse_error(status, &body_bytes));
            }

            let body_str = String::from_utf8_lossy(&body_bytes);
            self.parse_response(&body_str)
        })
        .await
    }

    async fn chat_stream(
        &self,
        request: ChatRequest,
        credentials: &Credentials,
    ) -> Result<ChatStream, ProviderError> {
        let provider = self.build_client(credentials)?;
        let (messages, system_texts) = self.build_messages(&request);
        let tools = request.tools.as_ref().map(|t| self.build_tools(t));

        let mut body = serde_json::json!({
            "model": request.model,
            "messages": messages,
            "max_tokens": request.max_tokens.unwrap_or(8192),
            "stream": true,
        });

        if let Some(tools) = tools {
            body["tools"] = serde_json::Value::Array(tools);
        }

        if !system_texts.is_empty() {
            body["system"] = serde_json::json!([{
                "type": "text",
                "text": system_texts.join("\n"),
                "cache_control": { "type": "ephemeral" }
            }]);
        }

        let url = format!("{}/v1/messages", provider.base_url);

        let (tx, rx) = tokio::sync::mpsc::channel(100);

        let provider_clone = provider.clone();
        let max_retries = self.max_retries;
        let retry_delay_ms = self.retry_delay_ms;

        tokio::spawn(async move {
            let mut last_error = None;
            for attempt in 0..=max_retries {
                let headers_result: Result<reqwest::header::HeaderMap, _> =
                    (&provider_clone.headers).try_into();
                let headers = match headers_result {
                    Ok(h) => h,
                    Err(e) => {
                        let _ = tx
                            .send(Err(ProviderError::Internal(format!(
                                "Invalid headers: {:?}",
                                e
                            ))))
                            .await;
                        return;
                    }
                };

                let response = match provider_clone
                    .client
                    .post(&url)
                    .headers(headers)
                    .json(&body)
                    .send()
                    .await
                {
                    Ok(r) => r,
                    Err(e) => {
                        let is_retryable = e.is_timeout() || e.is_connect() || e.is_request();
                        if !is_retryable || attempt == max_retries {
                            let _ = tx.send(Err(ProviderError::Network(e))).await;
                            return;
                        }
                        last_error = Some(ProviderError::Network(e));
                        let delay = Duration::from_millis(retry_delay_ms * 2u64.pow(attempt));
                        tracing::warn!(
                            "Streaming retryable error (attempt {}/{}): {:?}",
                            attempt + 1,
                            max_retries + 1,
                            last_error
                        );
                        sleep(delay).await;
                        continue;
                    }
                };

                if !response.status().is_success() {
                    let status = response.status().as_u16();
                    let body_bytes = response.bytes().await.unwrap_or_default();
                    if status == 429 && attempt < max_retries {
                        last_error = Some(ProviderError::RateLimited(
                            String::from_utf8_lossy(&body_bytes).to_string(),
                        ));
                        let delay = Duration::from_millis(retry_delay_ms * 2u64.pow(attempt));
                        tracing::warn!(
                            "Streaming rate limited (attempt {}/{}), retrying in {:?}",
                            attempt + 1,
                            max_retries + 1,
                            delay
                        );
                        sleep(delay).await;
                        continue;
                    }
                    let _ = tx
                        .send(Err(ProviderError::ProviderError {
                            status,
                            message: String::from_utf8_lossy(&body_bytes).to_string(),
                        }))
                        .await;
                    return;
                }

                let mut stream = response.bytes_stream();
                let mut buffer = String::new();
                let mut event_lines = Vec::new();

                while let Some(chunk) = stream.next().await {
                    match chunk {
                        Ok(bytes) => {
                            let text = String::from_utf8_lossy(&bytes);
                            buffer.push_str(&text);

                            while let Some(newline_pos) = buffer.find('\n') {
                                let line = buffer[..newline_pos].to_string();
                                buffer = buffer[newline_pos + 1..].to_string();
                                let trimmed = line.trim();

                                if trimmed.starts_with("data: ") {
                                    event_lines.push(trimmed.to_string());
                                } else if trimmed.is_empty() && !event_lines.is_empty() {
                                    if let Some(event) = Self::parse_stream_event(&event_lines)
                                        && tx.send(event).await.is_err()
                                    {
                                        return;
                                    }
                                    event_lines.clear();
                                }
                            }
                        }
                        Err(e) => {
                            let _ = tx.send(Err(ProviderError::Network(e))).await;
                            return;
                        }
                    }
                }

                if !event_lines.is_empty()
                    && let Some(event) = Self::parse_stream_event(&event_lines)
                {
                    let _ = tx.send(event).await;
                }

                return;
            }

            if let Some(e) = last_error {
                let _ = tx.send(Err(e)).await;
            }
        });

        Ok(ChatStream::new(rx))
    }

    fn required_headers(&self, credentials: &Credentials) -> HashMap<String, String> {
        let mut headers = HashMap::new();
        headers.insert("x-app".to_string(), "cli".to_string());
        headers.insert(
            "anthropic-version".to_string(),
            ANTHROPIC_API_VERSION.to_string(),
        );
        headers.insert("anthropic-beta".to_string(), self.beta_headers().join(","));

        match &credentials.auth_type {
            AuthType::ApiKey => {
                if let Some(key) = credentials.api_key_str() {
                    headers.insert("x-api-key".to_string(), key);
                }
            }
            AuthType::OAuth | AuthType::OAuthDerivedApiKey => {
                if let Some(token) = credentials.access_token_str() {
                    headers.insert("Authorization".to_string(), format!("Bearer {}", token));
                }
            }
        }

        headers
    }

    fn beta_headers(&self) -> Vec<String> {
        vec![
            "claude-code-20250219".to_string(),
            "interleaved-thinking-2025-05-14".to_string(),
            "context-1m-2025-08-07".to_string(),
            "structured-outputs-2025-12-15".to_string(),
            "web-search-2025-03-05".to_string(),
            "advanced-tool-use-2025-11-20".to_string(),
            "effort-2025-11-24".to_string(),
            "token-efficient-tools-2026-03-28".to_string(),
            "fast-mode-2026-02-01".to_string(),
        ]
    }

    fn extra_body_params(&self, _request: &ChatRequest) -> serde_json::Value {
        serde_json::Value::Object(serde_json::Map::new())
    }

    fn parse_error(&self, status: u16, body: &[u8]) -> ProviderError {
        let message = String::from_utf8_lossy(body).to_string();
        ProviderError::ProviderError { status, message }
    }

    async fn verify_credentials(&self, credentials: &Credentials) -> Result<(), ProviderError> {
        let client = self.build_client(credentials)?;
        let url = format!("{}/v1/messages", client.base_url);

        let response = client
            .client
            .post(&url)
            .headers(
                (&client.headers)
                    .try_into()
                    .map_err(|e| ProviderError::Internal(format!("{:?}", e)))?,
            )
            .json(&serde_json::json!({
                "model": "claude-haiku-4-5",
                "messages": [{"role": "user", "content": "test"}],
                "max_tokens": 1,
            }))
            .send()
            .await
            .map_err(ProviderError::Network)?;

        if response.status().is_success() {
            Ok(())
        } else {
            let status = response.status().as_u16();
            let body = response.bytes().await.unwrap_or_default();
            Err(self.parse_error(status, &body))
        }
    }

    async fn refresh_credentials(
        &self,
        credentials: &Credentials,
    ) -> Result<Credentials, ProviderError> {
        // OAuth refresh would go here — for now, return as-is
        Ok(credentials.clone())
    }

    async fn list_models(
        &self,
        _credentials: &Credentials,
    ) -> Result<Vec<ModelInfo>, ProviderError> {
        Ok(vec![
            ModelInfo {
                id: "claude-sonnet-4-5".to_string(),
                provider: "anthropic".to_string(),
                name: "Claude Sonnet 4.5".to_string(),
                context_window: 200_000,
                max_output_tokens: 8192,
                input_price_per_million: 3.0,
                output_price_per_million: 15.0,
                supports_tools: true,
                supports_vision: true,
                supports_reasoning: true,
            },
            ModelInfo {
                id: "claude-opus-4-5".to_string(),
                provider: "anthropic".to_string(),
                name: "Claude Opus 4.5".to_string(),
                context_window: 200_000,
                max_output_tokens: 8192,
                input_price_per_million: 15.0,
                output_price_per_million: 75.0,
                supports_tools: true,
                supports_vision: true,
                supports_reasoning: true,
            },
            ModelInfo {
                id: "claude-haiku-4-5".to_string(),
                provider: "anthropic".to_string(),
                name: "Claude Haiku 4.5".to_string(),
                context_window: 200_000,
                max_output_tokens: 8192,
                input_price_per_million: 0.8,
                output_price_per_million: 4.0,
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
