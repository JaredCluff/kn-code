use crate::traits::*;
use async_trait::async_trait;
use futures::StreamExt;
use kn_code_auth::Credentials;
use std::collections::HashMap;
use std::time::Duration;
use tokio::time::sleep;

#[derive(Debug)]
pub struct CompatibleProvider {
    pub name: String,
    pub base_url: String,
    pub models: Vec<String>,
    pub max_retries: u32,
    pub retry_delay_ms: u64,
}

impl CompatibleProvider {
    pub fn new(name: String, base_url: String, models: Vec<String>) -> Self {
        Self {
            name,
            base_url,
            models,
            max_retries: 3,
            retry_delay_ms: 1000,
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
                        "Compatible provider retryable error (attempt {}/{}): {:?}",
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

    fn build_messages(&self, request: &ChatRequest) -> Vec<serde_json::Value> {
        let mut messages = Vec::new();

        if let Some(system) = &request.system {
            messages.push(serde_json::json!({
                "role": "system",
                "content": system,
            }));
        }

        for msg in &request.messages {
            match &msg.role {
                MessageRole::User => {
                    let content: Vec<_> = msg
                        .content
                        .iter()
                        .map(|c| match c {
                            ContentBlock::Text(t) => {
                                serde_json::json!({"type": "text", "text": t})
                            }
                            _ => serde_json::json!({"type": "text", "text": ""}),
                        })
                        .collect();
                    messages.push(serde_json::json!({
                        "role": "user",
                        "content": content,
                    }));
                }
                MessageRole::Assistant => {
                    let mut content = Vec::new();
                    for block in &msg.content {
                        match block {
                            ContentBlock::Text(t) => {
                                content.push(serde_json::json!({"type": "text", "text": t}));
                            }
                            ContentBlock::ToolUse { id, name, input } => {
                                content.push(serde_json::json!({
                                    "type": "tool_calls",
                                    "tool_calls": [serde_json::json!({
                                        "id": id,
                                        "type": "function",
                                        "function": {
                                            "name": name,
                                            "arguments": input.to_string(),
                                        },
                                    })],
                                }));
                            }
                            _ => {}
                        }
                    }
                    let content_value = if content.len() == 1 {
                        let first = &content[0];
                        if first.get("type").and_then(|v| v.as_str()) == Some("text") {
                            first.get("text").cloned()
                        } else {
                            Some(serde_json::Value::Array(content))
                        }
                    } else {
                        Some(serde_json::Value::Array(content))
                    };
                    messages.push(serde_json::json!({
                        "role": "assistant",
                        "content": content_value,
                    }));
                }
                MessageRole::Tool => {
                    for block in &msg.content {
                        if let ContentBlock::ToolResult {
                            id,
                            content: text,
                            is_error: _,
                        } = block
                        {
                            messages.push(serde_json::json!({
                                "role": "tool",
                                "tool_call_id": id,
                                "content": text,
                            }));
                        }
                    }
                }
                MessageRole::System => {
                    for block in &msg.content {
                        if let ContentBlock::Text(t) = block {
                            messages.push(serde_json::json!({
                                "role": "system",
                                "content": t,
                            }));
                        }
                    }
                }
            }
        }

        messages
    }

    fn build_tools(&self, tools: &[ToolDefinition]) -> Vec<serde_json::Value> {
        tools
            .iter()
            .map(|t| {
                let mut schema = t.input_schema.clone();
                if schema.get("type").is_none() {
                    schema["type"] = serde_json::json!("object");
                }
                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": schema,
                    },
                })
            })
            .collect()
    }

    fn parse_response(&self, body: &str) -> Result<ChatResponse, ProviderError> {
        let json: serde_json::Value =
            serde_json::from_str(body).map_err(ProviderError::Serialization)?;

        let message = json
            .get("choices")
            .and_then(|c| c.as_array())
            .and_then(|arr| arr.first())
            .and_then(|c| c.get("message"))
            .ok_or_else(|| ProviderError::Internal("No choices in response".to_string()))?;

        let mut content = Vec::new();

        if let Some(text) = message.get("content").and_then(|v| v.as_str())
            && !text.is_empty() {
                content.push(ContentBlock::Text(text.to_string()));
            }

        if let Some(tool_calls) = message.get("tool_calls").and_then(|v| v.as_array()) {
            let empty_fn = serde_json::json!({});
            for tc in tool_calls {
                let id = tc
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let function = tc.get("function").unwrap_or(&empty_fn);
                let name = function
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let empty_args = serde_json::json!({});
                let input = function
                    .get("arguments")
                    .and_then(|v| v.as_str())
                    .and_then(|s| serde_json::from_str(s).ok())
                    .unwrap_or(empty_args);
                content.push(ContentBlock::ToolUse { id, name, input });
            }
        }

        let usage = json
            .get("usage")
            .map(|u| Usage {
                input_tokens: u.get("prompt_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
                output_tokens: u
                    .get("completion_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0),
                cached_input_tokens: 0,
            })
            .unwrap_or_default();

        let stop_reason = message
            .get("finish_reason")
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

    fn parse_stream_line(line: &str) -> Option<Result<StreamEvent, ProviderError>> {
        if !line.starts_with("data: ") {
            return None;
        }
        let data = &line[6..];
        if data == "[DONE]" {
            return None;
        }

        let json: serde_json::Value = match serde_json::from_str(data) {
            Ok(v) => v,
            Err(e) => return Some(Err(ProviderError::Serialization(e))),
        };

        let delta = json
            .get("choices")
            .and_then(|c| c.as_array())
            .and_then(|arr| arr.first())
            .and_then(|c| c.get("delta"))?;

        let mut events = Vec::new();

        if let Some(text) = delta.get("content").and_then(|v| v.as_str())
            && !text.is_empty() {
                events.push(Ok(StreamEvent::Text(text.to_string())));
            }

        if let Some(tool_calls) = delta.get("tool_calls").and_then(|v| v.as_array()) {
            for tc in tool_calls {
                if let (Some(id), Some(function)) =
                    (tc.get("id").and_then(|v| v.as_str()), tc.get("function"))
                {
                    let name = function
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let input = function
                        .get("arguments")
                        .and_then(|v| v.as_str())
                        .and_then(|s| serde_json::from_str(s).ok())
                        .unwrap_or(serde_json::json!({}));
                    events.push(Ok(StreamEvent::ToolUse {
                        id: id.to_string(),
                        name,
                        input,
                    }));
                }
            }
        }

        if let Some(usage) = json.get("usage") {
            let event_usage = Usage {
                input_tokens: usage
                    .get("prompt_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0),
                output_tokens: usage
                    .get("completion_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0),
                cached_input_tokens: 0,
            };
            events.push(Ok(StreamEvent::Usage(event_usage)));
        }

        if events.is_empty() {
            None
        } else {
            events.into_iter().next()
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
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(600))
                .build()
                .map_err(|e| {
                    ProviderError::Internal(format!("Failed to build HTTP client: {}", e))
                })?,
        })
    }

    async fn chat(
        &self,
        request: ChatRequest,
        credentials: &Credentials,
    ) -> Result<ChatResponse, ProviderError> {
        let provider = self.build_client(credentials)?;
        let messages = self.build_messages(&request);
        let tools = request.tools.as_ref().map(|t| self.build_tools(t));

        let mut body = serde_json::json!({
            "model": request.model,
            "messages": messages,
            "max_tokens": request.max_tokens.unwrap_or(8192),
        });

        if let Some(tools) = tools {
            body["tools"] = serde_json::Value::Array(tools);
            if request.tool_choice.is_some() {
                body["tool_choice"] = match &request.tool_choice {
                    Some(ToolChoice::Auto) => serde_json::json!("auto"),
                    Some(ToolChoice::Required) => serde_json::json!("required"),
                    Some(ToolChoice::None) => serde_json::json!("none"),
                    Some(ToolChoice::Specific(name)) => serde_json::json!({
                        "type": "function",
                        "function": { "name": name },
                    }),
                    None => serde_json::json!("auto"),
                };
            }
        }

        if let Some(temp) = request.temperature {
            body["temperature"] =
                serde_json::Value::Number(serde_json::Number::from_f64(temp as f64).unwrap());
        }

        let url = format!("{}/chat/completions", provider.base_url);

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
        let messages = self.build_messages(&request);
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

        let url = format!("{}/chat/completions", provider.base_url);

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
                            "Compatible streaming retryable error (attempt {}/{}): {:?}",
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
                            "Compatible streaming rate limited (attempt {}/{}), retrying in {:?}",
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

                while let Some(chunk) = stream.next().await {
                    match chunk {
                        Ok(bytes) => {
                            let text = String::from_utf8_lossy(&bytes);
                            buffer.push_str(&text);

                            while let Some(newline_pos) = buffer.find('\n') {
                                let line = buffer[..newline_pos].to_string();
                                buffer = buffer[newline_pos + 1..].to_string();
                                let trimmed = line.trim();

                                if !trimmed.is_empty()
                                    && let Some(event) = Self::parse_stream_line(trimmed)
                                        && tx.send(event).await.is_err() {
                                            return;
                                        }
                            }
                        }
                        Err(e) => {
                            let _ = tx.send(Err(ProviderError::Network(e))).await;
                            return;
                        }
                    }
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

    async fn verify_credentials(&self, credentials: &Credentials) -> Result<(), ProviderError> {
        let client = self.build_client(credentials)?;
        let url = format!("{}/chat/completions", client.base_url);

        let response = client
            .client
            .post(&url)
            .headers(
                (&client.headers)
                    .try_into()
                    .map_err(|e| ProviderError::Internal(format!("{:?}", e)))?,
            )
            .json(&serde_json::json!({
                "model": self.models.first().unwrap_or(&"gpt-4o".to_string()),
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
