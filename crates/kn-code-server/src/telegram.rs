use kn_code_voice::telegram::{TelegramConfig, TelegramVoiceMessage};
use kn_code_voice::wake_word::WakeWordDetector;
use secrecy::ExposeSecret;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramUpdate {
    pub update_id: i64,
    pub message: Option<TelegramMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramMessage {
    pub message_id: i64,
    pub chat: TelegramChat,
    pub from: Option<TelegramUser>,
    pub text: Option<String>,
    pub voice: Option<TelegramVoice>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramChat {
    pub id: i64,
    #[serde(rename = "type")]
    pub chat_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramUser {
    pub id: i64,
    pub username: Option<String>,
    pub first_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramVoice {
    pub file_id: String,
    pub duration: u32,
    pub mime_type: Option<String>,
}

pub struct TelegramBot {
    pub config: TelegramConfig,
    pub allowed_user_ids: Vec<i64>,
    pub wake_word_detector: Arc<WakeWordDetector>,
    pub client: reqwest::Client,
    pub last_update_id: Arc<RwLock<i64>>,
}

impl TelegramBot {
    pub fn new(
        config: TelegramConfig,
        wake_word_detector: WakeWordDetector,
        allowed_user_ids: Vec<i64>,
    ) -> Self {
        Self {
            config,
            allowed_user_ids,
            wake_word_detector: Arc::new(wake_word_detector),
            client: reqwest::Client::new(),
            last_update_id: Arc::new(RwLock::new(0)),
        }
    }

    pub async fn start_polling(&self) -> anyhow::Result<()> {
        tracing::info!("Telegram bot starting — polling for updates");

        let mut consecutive_errors = 0u32;
        const MAX_BACKOFF_SECS: u64 = 60;

        loop {
            match self.get_updates().await {
                Ok(updates) => {
                    consecutive_errors = 0;
                    for update in updates {
                        let last_id = *self.last_update_id.read().await;
                        if update.update_id <= last_id {
                            continue;
                        }
                        *self.last_update_id.write().await = update.update_id;

                        if let Some(msg) = update.message
                            && let Err(e) = self.handle_message(msg).await
                        {
                            tracing::error!("Failed to handle Telegram message: {}", e);
                        }
                    }
                }
                Err(e) => {
                    consecutive_errors = consecutive_errors.saturating_add(1);
                    tracing::error!("Failed to get Telegram updates: {}", e);
                    let backoff =
                        std::cmp::min(2u64.pow(consecutive_errors.min(6)), MAX_BACKOFF_SECS);
                    tracing::warn!(
                        backoff_secs = backoff,
                        consecutive_errors,
                        "Backing off Telegram polling"
                    );
                    tokio::time::sleep(std::time::Duration::from_secs(backoff)).await;
                    continue;
                }
            }

            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    }

    async fn get_updates(&self) -> anyhow::Result<Vec<TelegramUpdate>> {
        let url = format!(
            "https://api.telegram.org/bot{}/getUpdates",
            self.config.bot_token.expose_secret()
        );

        let last_id = *self.last_update_id.read().await;
        let response: reqwest::Response = self
            .client
            .get(&url)
            .query(&[
                ("offset", (last_id + 1).to_string()),
                ("timeout", "30".to_string()),
            ])
            .send()
            .await?;

        let json: serde_json::Value = response.json().await?;
        if !json.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
            anyhow::bail!("Telegram API error: {:?}", json.get("description"));
        }

        let updates: Vec<TelegramUpdate> = json
            .get("result")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| serde_json::from_value(v.clone()).ok())
                    .collect()
            })
            .unwrap_or_default();

        Ok(updates)
    }

    fn is_user_allowed(&self, user_id: i64) -> bool {
        if self.allowed_user_ids.is_empty() {
            return false;
        }
        self.allowed_user_ids.contains(&user_id)
    }

    async fn handle_message(&self, msg: TelegramMessage) -> anyhow::Result<()> {
        let chat_id = msg.chat.id;
        let user_id = msg.from.as_ref().map(|u| u.id).unwrap_or(0);

        if !self.is_user_allowed(user_id) {
            tracing::warn!("Unauthorized Telegram user {} attempted access", user_id);
            return Ok(());
        }

        if let Some(voice) = &msg.voice {
            let tg_msg = TelegramVoiceMessage {
                chat_id,
                message_id: msg.message_id,
                user_id,
                file_id: voice.file_id.clone(),
                duration_secs: voice.duration,
                mime_type: voice.mime_type.clone(),
            };

            tracing::info!("Voice message from {}: {:?}", user_id, tg_msg);
        } else if let Some(text) = &msg.text {
            if self.config.wake_word_required {
                if let Some(_word) = self.wake_word_detector.check_text_for_wake_word(text) {
                    let command = self.wake_word_detector.strip_wake_word(text);
                    tracing::info!("Text command from {}: {}", user_id, command);
                    let _ = self
                        .send_text_response(chat_id, &format!("Processing: {}", command))
                        .await;
                } else {
                    let _ = self
                        .send_text_response(
                            chat_id,
                            "No wake word detected. Try starting with a wake word.",
                        )
                        .await;
                }
            } else {
                tracing::info!("Text command from {}: {}", user_id, text);
                let _ = self
                    .send_text_response(chat_id, &format!("Processing: {}", text))
                    .await;
            }
        }

        Ok(())
    }

    pub async fn send_text_response(&self, chat_id: i64, text: &str) -> anyhow::Result<()> {
        let url = format!(
            "https://api.telegram.org/bot{}/sendMessage",
            self.config.bot_token.expose_secret()
        );

        let body = serde_json::json!({
            "chat_id": chat_id,
            "text": text,
        });

        let response: reqwest::Response = self.client.post(&url).json(&body).send().await?;
        if !response.status().is_success() {
            anyhow::bail!("Failed to send Telegram message: {}", response.status());
        }
        Ok(())
    }

    pub async fn send_message(&self, chat_id: i64, text: &str) -> anyhow::Result<()> {
        self.send_text_response(chat_id, text).await
    }
}
