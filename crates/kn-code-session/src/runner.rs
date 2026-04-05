use crate::messages::{
    AssistantMessage, ContentBlock as SessionContentBlock, Message, ToolCall, ToolMessage,
};
use crate::prompt::SystemPromptBuilder;
use crate::store::SessionStore;
use kn_code_auth::TokenStore;
use kn_code_permissions::rules::{PermissionContext, PermissionDecision, PermissionMode};
use kn_code_providers::traits::{
    ChatMessage as ProviderChatMessage, ChatRequest, ContentBlock as ProviderContentBlock,
    MessageRole, ModelInfo, Provider, ToolChoice, ToolDefinition,
};
use kn_code_tools::traits::{Tool, ToolContent, ToolContext};
use std::path::PathBuf;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

const MAX_TURNS: u64 = 100;

pub struct AgentRunner {
    pub session_store: Arc<SessionStore>,
    pub token_store: Arc<dyn TokenStore>,
    pub provider: Arc<dyn Provider>,
    pub tools: Vec<Arc<dyn Tool>>,
    pub permission_mode: PermissionMode,
    pub max_turns: u64,
    pub cwd: PathBuf,
    pub model_info: Option<ModelInfo>,
    pub cancellation_token: Option<CancellationToken>,
}

pub struct AgentRunResult {
    pub session_id: String,
    pub stop_reason: String,
    pub turns_completed: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
}

impl AgentRunner {
    pub async fn run(&self, session_id: &str) -> anyhow::Result<AgentRunResult> {
        let record = self
            .session_store
            .load_session(session_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Session not found: {}", session_id))?;

        let messages = self.session_store.load_messages(session_id).await?;
        if messages.is_empty() {
            return Err(anyhow::anyhow!("No messages in session"));
        }

        let model = record.model.clone();
        let mut all_messages = messages;
        let mut turns_completed = 0u64;
        let mut total_input_tokens = 0u64;
        let mut total_output_tokens = 0u64;

        let credentials = self
            .token_store
            .load(self.provider.id())
            .await?
            .ok_or_else(|| {
                anyhow::anyhow!("No credentials found for provider {}", self.provider.id())
            })?;

        let max_turns = self.max_turns.min(MAX_TURNS);

        let tool_descriptions: Vec<String> = self
            .tools
            .iter()
            .map(|t| {
                format!(
                    "## {}\n{}\n\nInput schema:\n```json\n{}\n```",
                    t.name(),
                    t.description(),
                    serde_json::to_string_pretty(&t.input_schema()).unwrap_or_default()
                )
            })
            .collect();

        let permission_mode_prompt = match self.permission_mode {
            PermissionMode::Auto => "You are in AUTO mode. All tool calls are automatically approved. Use tools freely.",
            PermissionMode::Ask => "You are in ASK mode. Each tool call requires user approval. Explain what you want to do before calling tools.",
            PermissionMode::AcceptEdits => "You are in ACCEPT_EDITS mode. File read/write/edit tools are auto-approved. Bash commands require approval.",
            PermissionMode::Plan => "You are in PLAN mode. Do NOT call any tools. Only provide analysis and recommendations.",
            PermissionMode::BypassPermissions => "All tool calls are automatically approved.",
        }
        .to_string();

        let system_prompt_builder = SystemPromptBuilder::new(self.cwd.clone());
        let system_prompt = system_prompt_builder
            .with_tool_descriptions(tool_descriptions)
            .with_permission_mode_prompt(permission_mode_prompt)
            .build_string()
            .await;

        for _turn in 0..max_turns {
            if let Some(ref token) = self.cancellation_token
                && token.is_cancelled()
            {
                self.session_store
                    .update_session_state(session_id, "cancelled")
                    .await?;
                return Ok(AgentRunResult {
                    session_id: session_id.to_string(),
                    stop_reason: "cancelled".to_string(),
                    turns_completed,
                    input_tokens: total_input_tokens,
                    output_tokens: total_output_tokens,
                    cost_usd: self.calculate_cost(total_input_tokens, total_output_tokens),
                });
            }

            if let Ok(Some(updated_record)) = self.session_store.load_session(session_id).await
                && updated_record.state == "cancelled"
            {
                return Ok(AgentRunResult {
                    session_id: session_id.to_string(),
                    stop_reason: "cancelled".to_string(),
                    turns_completed,
                    input_tokens: total_input_tokens,
                    output_tokens: total_output_tokens,
                    cost_usd: self.calculate_cost(total_input_tokens, total_output_tokens),
                });
            }

            let provider_messages = session_messages_to_provider(&all_messages)?;
            let tool_defs: Vec<ToolDefinition> = self
                .tools
                .iter()
                .map(|t| ToolDefinition {
                    name: t.name().to_string(),
                    description: t.description().to_string(),
                    input_schema: t.input_schema(),
                })
                .collect();

            let request = ChatRequest {
                model: model.clone(),
                messages: provider_messages,
                tools: if tool_defs.is_empty() {
                    None
                } else {
                    Some(tool_defs)
                },
                tool_choice: Some(ToolChoice::Auto),
                temperature: None,
                top_p: None,
                max_tokens: None,
                stream: false,
                system: Some(system_prompt.clone()),
                variant: None,
            };

            let response = self.provider.chat(request, &credentials).await?;

            total_input_tokens += response.usage.input_tokens;
            total_output_tokens += response.usage.output_tokens;

            let mut assistant_content = Vec::new();
            let mut assistant_tool_calls = Vec::new();

            for block in &response.content {
                match block {
                    ProviderContentBlock::Text(t) => {
                        assistant_content.push(SessionContentBlock::Text(t.clone()));
                    }
                    ProviderContentBlock::ToolUse { id, name, input } => {
                        assistant_tool_calls.push(ToolCall {
                            id: id.clone(),
                            name: name.clone(),
                            input: input.clone(),
                        });
                    }
                    ProviderContentBlock::Thinking { text } => {
                        assistant_content.push(SessionContentBlock::Text(format!(
                            "<thinking>{}</thinking>",
                            text
                        )));
                    }
                    _ => {}
                }
            }

            let assistant_msg = Message::Assistant(AssistantMessage {
                id: uuid::Uuid::new_v4().to_string(),
                content: assistant_content,
                tool_calls: assistant_tool_calls.clone(),
                model: model.clone(),
                stop_reason: response.stop_reason.clone(),
                timestamp: chrono::Utc::now(),
            });

            self.session_store
                .append_message(session_id, &assistant_msg)
                .await?;
            all_messages.push(assistant_msg);

            let stop_reason = response.stop_reason.as_deref().unwrap_or("end_turn");

            if assistant_tool_calls.is_empty() {
                let state = if stop_reason == "max_tokens" {
                    "max_tokens_reached"
                } else {
                    "completed"
                };
                self.session_store
                    .update_session_state(session_id, state)
                    .await?;

                if stop_reason == "tool_use" {
                    tracing::warn!(
                        session_id,
                        "Provider returned stop_reason=tool_use but no tool calls — response may be truncated"
                    );
                }

                return Ok(AgentRunResult {
                    session_id: session_id.to_string(),
                    stop_reason: stop_reason.to_string(),
                    turns_completed: turns_completed + 1,
                    input_tokens: total_input_tokens,
                    output_tokens: total_output_tokens,
                    cost_usd: self.calculate_cost(total_input_tokens, total_output_tokens),
                });
            }

            if stop_reason == "max_tokens" {
                tracing::warn!(
                    session_id,
                    "Provider returned stop_reason=max_tokens with pending tool calls"
                );
            }

            for tool_call in &assistant_tool_calls {
                let tool = self.tools.iter().find(|t| {
                    t.name() == tool_call.name || t.aliases().contains(&tool_call.name.as_str())
                });

                let (content, is_error) = match tool {
                    Some(t) => {
                        let _perm_ctx = PermissionContext {
                            mode: self.permission_mode,
                            additional_working_directories: vec![self.cwd.clone()],
                            ..Default::default()
                        };

                        let decision = t
                            .check_permission(
                                &tool_call.input,
                                &ToolContext {
                                    cwd: self.cwd.clone(),
                                    is_headless: true,
                                    session_id: Some(session_id.to_string()),
                                    tool_use_id: tool_call.id.clone(),
                                },
                            )
                            .await?;

                        match decision {
                            PermissionDecision::Allow { .. } => {
                                match t
                                    .call(
                                        tool_call.input.clone(),
                                        ToolContext {
                                            cwd: self.cwd.clone(),
                                            is_headless: true,
                                            session_id: Some(session_id.to_string()),
                                            tool_use_id: tool_call.id.clone(),
                                        },
                                    )
                                    .await
                                {
                                    Ok(result) => match result.content {
                                        ToolContent::Text(t) => (t, false),
                                        ToolContent::Image {
                                            base64: _,
                                            media_type,
                                        } => (format!("[Image: {}]", media_type), false),
                                        ToolContent::Multi(blocks) => {
                                            let parts: Vec<_> = blocks
                                                .iter()
                                                .filter_map(|b| match b {
                                                    kn_code_tools::traits::ContentBlock::Text(t) => {
                                                        Some(t.clone())
                                                    }
                                                    kn_code_tools::traits::ContentBlock::Error {
                                                        message,
                                                        ..
                                                    } => Some(format!("[Error: {}]", message)),
                                                    _ => None,
                                                })
                                                .collect();
                                            (parts.join("\n"), false)
                                        }
                                    },
                                    Err(e) => (e.to_string(), true),
                                }
                            }
                            PermissionDecision::Deny { message, .. } => {
                                (format!("Permission denied: {}", message), true)
                            }
                            PermissionDecision::Ask { .. } => (
                                format!(
                                    "Tool '{}' requires manual approval (interactive mode not supported)",
                                    tool_call.name
                                ),
                                true,
                            ),
                            PermissionDecision::Passthrough { .. } => (
                                format!(
                                    "Tool '{}' requires permission (passthrough mode not supported in headless)",
                                    tool_call.name
                                ),
                                true,
                            ),
                        }
                    }
                    None => (format!("Unknown tool: {}", tool_call.name), true),
                };

                let tool_result_msg = Message::Tool(ToolMessage {
                    id: uuid::Uuid::new_v4().to_string(),
                    tool_use_id: tool_call.id.clone(),
                    tool_name: tool_call.name.clone(),
                    input: tool_call.input.clone(),
                    output: content,
                    is_error,
                    duration_ms: None,
                    timestamp: chrono::Utc::now(),
                });

                self.session_store
                    .append_message(session_id, &tool_result_msg)
                    .await?;
                all_messages.push(tool_result_msg);
            }

            turns_completed += 1;
        }

        self.session_store
            .update_session_state(session_id, "max_turns_reached")
            .await?;

        Ok(AgentRunResult {
            session_id: session_id.to_string(),
            stop_reason: "max_turns".to_string(),
            turns_completed,
            input_tokens: total_input_tokens,
            output_tokens: total_output_tokens,
            cost_usd: self.calculate_cost(total_input_tokens, total_output_tokens),
        })
    }

    fn calculate_cost(&self, input_tokens: u64, output_tokens: u64) -> f64 {
        if let Some(info) = &self.model_info {
            let input_cost = (input_tokens as f64 / 1_000_000.0) * info.input_price_per_million;
            let output_cost = (output_tokens as f64 / 1_000_000.0) * info.output_price_per_million;
            input_cost + output_cost
        } else {
            0.0
        }
    }
}

fn session_messages_to_provider(messages: &[Message]) -> anyhow::Result<Vec<ProviderChatMessage>> {
    let mut result = Vec::new();

    for msg in messages {
        match msg {
            Message::User(u) => {
                result.push(ProviderChatMessage {
                    role: MessageRole::User,
                    content: u
                        .content
                        .iter()
                        .filter_map(|b| match b {
                            SessionContentBlock::Text(t) => {
                                Some(ProviderContentBlock::Text(t.clone()))
                            }
                            _ => None,
                        })
                        .collect(),
                });
            }
            Message::Assistant(a) => {
                let mut content = Vec::new();
                for block in &a.content {
                    if let SessionContentBlock::Text(t) = block {
                        content.push(ProviderContentBlock::Text(t.clone()));
                    }
                }
                for tc in &a.tool_calls {
                    content.push(ProviderContentBlock::ToolUse {
                        id: tc.id.clone(),
                        name: tc.name.clone(),
                        input: tc.input.clone(),
                    });
                }
                result.push(ProviderChatMessage {
                    role: MessageRole::Assistant,
                    content,
                });
            }
            Message::Tool(t) => {
                result.push(ProviderChatMessage {
                    role: MessageRole::Tool,
                    content: vec![ProviderContentBlock::ToolResult {
                        id: t.tool_use_id.clone(),
                        content: t.output.clone(),
                        is_error: t.is_error,
                    }],
                });
            }
            Message::System(s) => {
                result.push(ProviderChatMessage {
                    role: MessageRole::System,
                    content: vec![ProviderContentBlock::Text(s.content.clone())],
                });
            }
        }
    }

    Ok(result)
}
