use crate::messages::Message;
use crate::store::SessionRecord;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Recovery state for an interrupted session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryState {
    pub session_id: String,
    pub last_activity: DateTime<Utc>,
    pub turns_completed: u64,
    pub last_message_type: String,
    pub pending_tool_calls: Vec<PendingToolCall>,
    pub recovery_summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingToolCall {
    pub id: String,
    pub name: String,
    pub input: serde_json::Value,
    pub has_result: bool,
}

/// Conversation recovery manager.
///
/// When a session is interrupted:
/// 1. Load the session from disk
/// 2. Determine what was in progress (pending tool calls, incomplete turns)
/// 3. Build a recovery summary
/// 4. Optionally replay incomplete tool calls
/// 5. Add a recovery handoff message
pub struct RecoveryManager {
    pub session_dir: PathBuf,
}

impl RecoveryManager {
    pub fn new(session_dir: PathBuf) -> Self {
        Self { session_dir }
    }

    /// Analyze a session for recovery.
    pub async fn analyze(
        &self,
        session: &SessionRecord,
        messages: &[Message],
    ) -> anyhow::Result<RecoveryState> {
        let last_message = messages.last();
        let last_message_type = last_message
            .map(|m| match m {
                Message::User(_) => "user",
                Message::Assistant(_) => "assistant",
                Message::Tool(_) => "tool",
                Message::System(_) => "system",
            })
            .unwrap_or("none")
            .to_string();

        // Find pending tool calls (tool_use without corresponding tool_result)
        let mut pending_tool_calls = Vec::new();
        let mut tool_use_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut tool_result_ids: std::collections::HashSet<String> =
            std::collections::HashSet::new();

        for msg in messages {
            match msg {
                Message::Assistant(assistant) => {
                    for tc in &assistant.tool_calls {
                        tool_use_ids.insert(tc.id.clone());
                        pending_tool_calls.push(PendingToolCall {
                            id: tc.id.clone(),
                            name: tc.name.clone(),
                            input: tc.input.clone(),
                            has_result: false,
                        });
                    }
                }
                Message::Tool(tool_msg) => {
                    tool_result_ids.insert(tool_msg.tool_use_id.clone());
                }
                _ => {}
            }
        }

        // Mark completed tool calls
        for pending in &mut pending_tool_calls {
            if tool_result_ids.contains(&pending.id) {
                pending.has_result = true;
            }
        }

        // Filter to only truly pending calls
        pending_tool_calls.retain(|tc| !tc.has_result);

        let recovery_summary = if pending_tool_calls.is_empty() {
            match last_message_type.as_str() {
                "assistant" => Some(
                    "Session was interrupted after assistant response. Safe to resume.".to_string(),
                ),
                "user" => Some(
                    "Session was interrupted after user message. Resuming processing.".to_string(),
                ),
                _ => None,
            }
        } else {
            let call_names: Vec<_> = pending_tool_calls
                .iter()
                .map(|tc| tc.name.clone())
                .collect();
            Some(format!(
                "Session was interrupted with {} pending tool call(s): {}. These will be re-executed on resume.",
                pending_tool_calls.len(),
                call_names.join(", ")
            ))
        };

        Ok(RecoveryState {
            session_id: session.id.clone(),
            last_activity: session.updated_at,
            turns_completed: session.turns_completed,
            last_message_type,
            pending_tool_calls,
            recovery_summary,
        })
    }

    /// Build a recovery handoff message to add when resuming.
    pub fn build_handoff_message(&self, state: &RecoveryState) -> Message {
        let content = format!(
            "CONVERSATION RECOVERY\n\
             Session {} was interrupted after {} turns.\n\
             Last message type: {}\n\
             Pending tool calls: {}\n\
             {}",
            state.session_id,
            state.turns_completed,
            state.last_message_type,
            state.pending_tool_calls.len(),
            state
                .recovery_summary
                .as_deref()
                .unwrap_or("Unknown state."),
        );

        Message::System(crate::messages::SystemMessage {
            id: format!("recovery_{}", state.session_id),
            content,
            subtype: "recovery".to_string(),
            timestamp: Utc::now(),
        })
    }
}
