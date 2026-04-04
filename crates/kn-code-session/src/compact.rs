use crate::messages::{ContentBlock, Message, SystemMessage};

pub struct Compactor {
    pub max_tokens: usize,
    pub target_tokens: usize,
}

impl Default for Compactor {
    fn default() -> Self {
        Self {
            max_tokens: 180_000,
            target_tokens: 80_000,
        }
    }
}

impl Compactor {
    pub fn needs_compaction(&self, current_tokens: usize) -> bool {
        current_tokens > self.max_tokens
    }

    pub fn compact(&self, messages: Vec<Message>) -> anyhow::Result<Vec<Message>> {
        if messages.len() <= 2 {
            return Ok(messages);
        }

        let mut compacted = Vec::new();

        // Keep all leading System messages (the system prompt)
        let mut first_non_system = 0;
        for msg in &messages {
            if matches!(msg, Message::System(_)) {
                compacted.push(msg.clone());
                first_non_system += 1;
            } else {
                break;
            }
        }

        let total = messages.len();
        let keep_recent = 4.min(total.saturating_sub(first_non_system));

        let mut summary_parts = Vec::new();
        for msg in messages
            .iter()
            .skip(first_non_system)
            .take(total - keep_recent - first_non_system)
        {
            match msg {
                Message::User(u) => {
                    for block in &u.content {
                        if let ContentBlock::Text(t) = block {
                            let preview = if t.len() > 200 {
                                format!("{}...", &t[..200])
                            } else {
                                t.clone()
                            };
                            summary_parts.push(format!("[User] {}", preview));
                        }
                    }
                }
                Message::Assistant(a) => {
                    for block in &a.content {
                        if let ContentBlock::Text(t) = block {
                            let preview = if t.len() > 200 {
                                format!("{}...", &t[..200])
                            } else {
                                t.clone()
                            };
                            summary_parts.push(format!("[Assistant] {}", preview));
                        }
                    }
                    if !a.tool_calls.is_empty() {
                        summary_parts
                            .push(format!("[Assistant called {} tool(s)]", a.tool_calls.len()));
                    }
                }
                Message::Tool(t) => {
                    let output_preview = if t.output.len() > 200 {
                        format!("{}...", &t.output[..200])
                    } else {
                        t.output.clone()
                    };
                    summary_parts.push(format!("[Tool {} result] {}", t.tool_name, output_preview));
                }
                Message::System(s) => {
                    summary_parts.push(format!("[System] {}", s.content));
                }
            }
        }

        if !summary_parts.is_empty() {
            let summary = format!(
                "Previous conversation summary ({} messages condensed):\n{}",
                total - keep_recent - first_non_system,
                summary_parts.join("\n")
            );
            compacted.push(Message::System(SystemMessage {
                id: uuid::Uuid::new_v4().to_string(),
                content: summary,
                subtype: "compaction_summary".to_string(),
                timestamp: chrono::Utc::now(),
            }));
        }

        for msg in messages.iter().skip(total - keep_recent) {
            compacted.push(msg.clone());
        }

        tracing::info!("Compacted {} messages down to {}", total, compacted.len());

        Ok(compacted)
    }
}
